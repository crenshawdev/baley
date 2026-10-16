# Baley security review rules

## Threat model

Baley is one Rust binary. Claude Code starts it as a stdio MCP server (`baley serve`) and as a pre-tool-use hook (`baley guard`), and the owner runs its command line. All three run as the interactive user, outside Claude Code's sandbox. It keeps a hash-chained SQLite ledger in Baley's home folder.

The main attacker is the AI agent in the owner's session, along with any text injected into it. Its goals are to:

- approve its own work or get any act recorded as the owner's;
- change the policy that judges it;
- commit or push to a protected branch;
- tamper with the ledger;
- run code outside the sandbox.

Outside attackers come second: the update source, the network, a forge holding anchor tags, and a hostile cloned repository.

The control everything rests on is the sandbox block and the `Read` and `Edit` deny rules that `baley install` writes into Claude Code's `settings.json` (`crates/baley/src/host_artifacts/security.rs`). They deny agents reads and writes of Baley's home and config folders, and writes of the executable, the versions folder and the placed files. Nothing else tells the owner at a terminal apart from an agent running the same command. The owner's recorded name (`owner.name`, or git `user.name`) is a label, not authentication.

Ways in:

- MCP frames over stdio;
- the hook's JSON on stdin;
- CLI arguments and stdin;
- environment variables (`CLAUDE_PROJECT_DIR`, `CLAUDE_CONFIG_DIR`, `BALEY_HOME`, `XDG_*`, `HOME`, `PATH`);
- the committed `baley.toml`, whose policy is HEAD's copy, and the owner's `config.toml`;
- git output and the repository's own `.git/config`;
- the update manifest and binary;
- anchor tags read back from the forge.

Out of scope: the inherited engine listed in `crates/baley/src/inherited.rs`, which production does not reach. Flag any change that makes production reach it.

## Review checklist

**Owner acts and the sandbox**
- An owner act (an approval of scope, truths or plans) must be served on the command line only, never over MCP. A new `baley_apply` or `baley_query` operation that records `Actor::Owner` is a critical finding.
- Over MCP only `capture` and `scope-submit` may write the ledger. A new MCP write path needs the same project-in-ledger check as `crates/baley/src/mcp/prepare.rs`.
- Install must render `sandbox.enabled = true`, `failIfUnavailable = true`, `allowUnsandboxedCommands = false` and the deny lists. It must refuse a settings document that would weaken them (`crates/baley/src/host_artifacts/compose.rs`). Flag any change that drops a deny entry, adds an exclusion, or accepts an `allowRead` or `allowWrite` entry that reaches Baley's folders.
- `baley doctor` must keep reporting drift in those settings and what an agent can reach.

**The guard (`crates/baley/src/guard_hook/`, `crates/baley-core/src/guard/`)**
- Design 0010 states these limits, so they are not findings:
  - only `git commit` and `git push` are scanned, and a declined command shape passes;
  - `PowerShell` is asked about, never scanned;
  - command-tool input over 64 KiB passes;
  - a failed git read passes unless `git.guard_hard_fail` is set.
- A command shape the scan claims to recognise but judges wrongly is a finding.
- The path tools (`Read`, `Grep`, `Glob`, `Write`, `Edit`, `NotebookEdit`) must fail closed. Unreadable input, an unresolvable path, or protected paths that cannot be derived must give deny.
- The guard must launch nothing but git, stay inside its time budget, and never record command text.

**Processes (`crates/baley/src/process.rs`, `git_process.rs`)**
- Every child starts at `process.rs`. A new launch of a program other than git, `claude` or Baley's own binary needs a reason.
- A git argument built from data must not be readable as an option. Use `--`, `--end-of-options` or a leading-dash check.
- Git runs outside the sandbox in an agent-writable checkout and honours its `.git/config`. Flag new git calls that can run configured programs: network operations, operations that refresh the index (fsmonitor), or a hook-running command without `--no-verify`.
- Never run a shell, and never interpolate data into a command string.

**Files**
- Writes go through `crates/baley/src/replace.rs` or an equivalent that refuses symlinks, checks the target is unchanged, and renames atomically.
- Install changes only what it can prove it wrote, by hash or exact match against the install record.
- The ledger home must stay 0700 with 0600 files, owned by the user, and every open must check that.

**Ledger (`crates/baley-store`, `crates/baley-store-sqlite`)**
- SQL binds values as parameters. `format!` SQL may insert only identifiers checked by `safe_identifier` or compiled constants.
- Events stay append-only and hash-chained. A path that writes events without extending the chain, or that rewrites or deletes outside `purge`, is a finding.
- `baley-core` and `baley-store` must not depend on rusqlite.

**Network and updates (`crates/baley/src/update/`, `crates/baley/src/ledger/forge.rs`)**
- Downloads stay HTTPS-only with no proxy and no credentials, within their size and time caps, and are checked against the manifest's SHA-256 before they are staged or run. The manifest being unsigned is tracked as #14.
- The only data that leaves the machine is anchor tags pushed to `git.remote`'s push URL, and the update downloads from `updates.source`. Any other network call, or telemetry of any kind, is a finding.

**Secrets**
- Baley must never read, store, send or log API keys, and must never call a model provider. Keys stay in the owner's environment.
- Remote URLs and git stderr must have userinfo stripped before they are recorded or printed (`crates/baley/src/checkout/strip.rs`).
- Guard records keep the input digest, never the command.
- New text an agent supplies that is stored or echoed back needs bounds and must not carry secrets into new places.

**Known and tracked, not findings:** #14 (unsigned manifests), #30 (the parked review engine still compiled in), #26 (file lease not enforced), #27 (anchors pushed by hand), #24 (macOS not qualified).
