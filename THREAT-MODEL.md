# Threat model: Baley

Version or commit this describes: `daa2416e` on `main`
Last reviewed: 2026-10-15

Baley is one Rust binary that keeps AI coding agents accountable to the owner who answers for their work. Each Claude Code session starts it as an MCP server over stdio, a pre-tool-use hook runs it before each shell or file tool call, and the owner runs its command line at a terminal. It keeps an append-only, hash-chained event ledger in SQLite outside the repository.

Every claim below was read from the code at the commit above. A claim marked `[VERIFY]` was not confirmed and needs checking.

## Scope of an audit

In scope: everything reachable from `crates/baley/src/main.rs` (the `serve` and `guard` modes and the command-line subcommands), the library crates `baley-core`, `baley-store` and `baley-store-sqlite`, and `install.sh`.

Out of scope:

- The inherited engine that `crates/baley/src/inherited.rs` lists, and code only it reaches: the review engine and its provider clients, the test runner, landing, the rail, pause, undo, milestone, why, recall and the transcript reader. Production does not reach it and Build 9 ([#30](https://github.com/crenshawdev/baley/issues/30)) deletes it. One check stays in scope: that nothing reachable from `main.rs` calls into it. `crates/baley/src/review/provider/credentials.rs` in particular still reads `OPENAI_API_KEY`, `GEMINI_API_KEY`, `DEEPSEEK_API_KEY` and a `providers.env` file, and must stay unreachable.
- `crates/baley-bench`, `spikes/`, and test code.

## Attackers

The main attacker is the AI agent working in the owner's Claude Code session, its main session or any subagent, together with any text injected into it through web pages, files or tool output. It already runs shell commands and edits files in the project. Its goals are to get around the process:

- approve or rule on its own work, or get any act recorded as the owner's;
- change the policy that judges it (protected branches, the guard's settings, the sandbox settings);
- commit or push to a protected branch without the owner deciding;
- tamper with the ledger, or with what the ledger says happened;
- reach the owner's API keys or other credentials.

Second come outside attackers: whoever controls the update source or the network path to it, a forge that holds anchor tags, and a cloned repository whose committed files are hostile.

## What this tool reads

**Baley's folders.** `crates/baley/src/folders.rs:45-133` resolves them from `BALEY_HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `HOME`. `BALEY_HOME` must be absolute and becomes both folders. Otherwise:

| Folder | Linux | macOS |
|---|---|---|
| Config | `${XDG_CONFIG_HOME:-~/.config}/crenshawdev/baley` | `~/Library/Application Support/crenshawdev/baley` |
| Home (data) | `${XDG_DATA_HOME:-~/.local/share}/crenshawdev/baley` | the same folder as config |

An empty or relative XDG value is ignored.

**Files.**

- `<config>/config.toml`, the owner's global settings. It is read whole and follows symlinks with no size cap (`crates/baley/src/settings.rs:72-109`). The guard caps it at 1 MiB (`settings.rs:44-57`).
- The ledger: `<home>/baley.db` with its `-wal`, `-shm`, `.writer` and `.maintenance` files (`crates/baley-store-sqlite/src/checks.rs:8-14`).
- The project's `baley.toml`, in two copies. The working-tree copy gives the project id (`crates/baley/src/mcp/prepare.rs:331-339`), and policy comes from HEAD's committed copy, read through git (`crates/baley/src/committed.rs`, `crates/baley/src/policy_step/read.rs:39-62`).
- Each ancestor of the working directory or `CLAUDE_PROJECT_DIR`, lstat'ed for `baley.toml` and `.git` up to the repository root (`crates/baley/src/discovery.rs:44-97`).
- `.git`, a gitdir file and `HEAD`, read directly by the guard as a fallback when git gives no branch. These reads use `O_NOFOLLOW` and are capped at 4 KiB (`crates/baley/src/guard_hook/branch.rs:77-138`).
- Claude Code's files, under `$CLAUDE_CONFIG_DIR` or `~/.claude`: `settings.json`, `skills/<id>/SKILL.md`, and `.claude.json` (in that folder when `CLAUDE_CONFIG_DIR` is set, otherwise in `HOME`) (`crates/baley/src/host_artifacts/installed.rs:65-110`). Install, the doctor and the interview read them whole, following symlinks, with no size cap (`crates/baley/src/host_doctor/placed.rs:64-95`). The guard only derives these paths and reads none of them.

**Git.** Every git read runs the `git` found on `PATH`:

- `rev-parse`, `ls-tree` and `cat-file` for HEAD's `baley.toml` (`committed.rs:144-199`);
- `rev-parse`, `rev-list --max-parents=0` and `remote get-url` for checkout facts (`crates/baley/src/checkout/gather.rs`);
- `symbolic-ref` for the guard's branch (`guard_hook/branch.rs:25-42`);
- `config --get user.name` for the owner's name (`crates/baley/src/providers/identity.rs:21`);
- for anchors, `remote`, `remote get-url --push --all`, `ls-remote` and `fetch` (`crates/baley/src/ledger/forge.rs`).

**Environment.**

- `CLAUDE_PROJECT_DIR`: absolute, UTF-8, at most 4,096 bytes (`crates/baley/src/mcp/context.rs:170-182`).
- `CLAUDE_CODE_SESSION_ID`: at most 128 bytes.
- `CLAUDE_CONFIG_DIR`, `BALEY_HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `HOME` and `PATH`.

No clap argument reads the environment.

**Standard input.**

- In `baley serve`, MCP frames.
- In `baley guard`, Claude Code's hook JSON, read up to 64 KiB plus one byte (`crates/baley/src/hook_input/mod.rs:21`).
- `baley models import` reads curl's output, at most 4 MiB plus a trailer (`crates/baley/src/models.rs:546-565`).
- `baley skill-description` reads markdown with no limit (`main.rs:148-151`).
- `baley config interview` reads answers.

**Network.** The update manifest and binary from `updates.source`, and anchor tags read back through git from the remote that `git.remote` names. The section on network responses below gives the limits.

**Not read by reachable code:** transcripts under `~/.claude/projects`, `.mcp.json`, a project's `.claude/` folder, `settings.local.json`, `CLAUDE.md`, agent files, `providers.env`, API key variables and `/proc`. An old `keys.env` in the config folder is never read (`folders.rs:59`).

**Other users.** Every ledger open refuses a home or file that is a link, owned by another user, or has a mode wider than 0700 for the folder and 0600 for the files (`crates/baley-store-sqlite/src/checks.rs:160-188`).

## What it writes

| Path | How and with what mode | Where |
|---|---|---|
| `<home>`, `<config>` | The final folder is created 0700. Missing parents get the default mode, and an existing folder's mode is left alone. | `crates/baley/src/folders.rs:156-184` |
| `<home>/baley.db` | Created `create_new` with mode 0600, in WAL mode, with `secure_delete = ON` | `crates/baley-store-sqlite/src/store.rs:255-262, 631, 724` |
| `<home>/baley.db-wal`, `-shm` | Created by the bundled SQLite. Baley sets no mode, but checks 0600 on the next open. `[VERIFY]` the mode SQLite gives them on creation. | |
| `<home>/baley.db.writer`, `.maintenance` | Created 0600 after the home check, without `O_NOFOLLOW` | `crates/baley-store-sqlite/src/queue.rs:46-52` |
| `<home>/install.lock` | 0600, `O_NOFOLLOW`, flock | `crates/baley/src/install/command.rs:102-116` |
| `<config>/config.toml` | Atomic replace (see below) | `crates/baley/src/config_command/set.rs:158, 241` |
| `<repo root>/baley.toml` | Written by `baley init` and by `baley config set` on the project layer. A new file gets the umask mode, and an existing one keeps its mode. | `crates/baley/src/init.rs:226-238`, `crates/baley/src/replace.rs:141-148` |
| Git objects in the repository | `baley anchor` writes an empty tree and a tag object (`hash-object -w`, `mktag`). Fetching an anchor writes objects only: no refs and no `FETCH_HEAD`. | `crates/baley/src/ledger/forge.rs:189-197, 311-321` |
| `baley ledger export --to <dir>` | Refuses an existing path. Creates the folder 0700 and its files 0600. | `crates/baley-store-sqlite/src/export.rs:72-75, 135-155` |
| `~/.local/lib/crenshawdev/baley/versions/<v>/baley` | Written to a `create_new` temporary file, set to 0755, fsynced, then renamed. The version folder is 0755. | `crates/baley/src/update/deliver.rs:93-157, 510-520` |
| `~/.local/bin/baley` | A symlink. The binary never creates it (`install.sh` does), and `baley update` swaps it with `renameat2(RENAME_EXCHANGE)`, verifies the new target, and restores the old one on failure. | `crates/baley/src/update/deliver.rs:159-195, 325-401` |

**Outside its own folders.**

- **Claude Code's `settings.json`.** Install writes four things into it:
  - one `PreToolUse` hook (described under What it executes);
  - the sandbox block: `sandbox.enabled = true`, `sandbox.failIfUnavailable = true`, `sandbox.allowUnsandboxedCommands = false`, with `sandbox.filesystem.denyRead` on the home and config folders and `sandbox.filesystem.denyWrite` on those folders, the versions folder, the placed files and the executable;
  - `permissions.deny` `Read` and `Edit` rules on the same folders and files (`crates/baley/src/host_artifacts/security.rs:146-222`);
  - the providers' API hosts in `sandbox.network.allowedDomains`, which the interview adds (`crates/baley/src/install/hosts.rs:58-82`).
- **The MCP registration.** Install runs `claude mcp add-json --scope user baley <json>`, and sometimes `claude mcp remove --scope user baley` first. Claude Code writes `.claude.json`, and Baley never writes it directly (`crates/baley/src/install/registration.rs:40-57`).
- **Skill stubs.** Install writes `skills/<id>/SKILL.md` under the Claude Code folder, whose folders it creates with the default mode (`crates/baley/src/install/apply.rs:60-81`).
- **The forge.** `baley anchor` pushes `refs/tags/baley-anchor/<project>/<seq>`.
- **Nothing else.** Nothing reachable writes shell rc files, systemd, launchd or cron units, git config or local refs.

**How writes are done.** `crates/baley/src/replace.rs:121-187` is the atomic replace. It writes a `create_new` temporary file in the same folder, copies the target's mode, and fsyncs. It then re-reads the target with `O_NOFOLLOW` and refuses if the target changed, appeared, vanished or is a link. Last it renames over the target and fsyncs the folder. Only the final path component is checked, so parent folders are followed. Writes through a link are refused everywhere (`install/hosts.rs:109-112`, `install/stubs.rs:44-49`, `config_command/set.rs:592-600`).

Install changes only what it can prove it wrote:

- a stub, only when its bytes match a hash in the install record;
- the registration, only when it equals the recorded entry;
- older Baley settings entries, only on an exact match.

Any conflict refuses the whole plan (`crates/baley/src/install/plan.rs:312`). The update refuses an existing occupant unless it holds identical bytes. One write path is weaker: `create_folder` in `update/deliver.rs:518` accepts an existing path whose `is_dir()` follows a link.

**Standard output and error.**

| Mode | Standard output | Standard error |
|---|---|---|
| `serve` | MCP JSON-RPC frames only. Tool results go into the model's context. | `baley:` diagnostics. `[VERIFY]` where Claude Code shows them. |
| `guard` | One `hookSpecificOutput` JSON object with `permissionDecision` `ask` or `deny` and a reason of at most 10,000 bytes, exit 0. A pass prints nothing. A deny reason reaches the model, and an ask reaches the owner. | A pass on failure prints one line, exit 0, and Claude Code sends that line only to its debug log. When standard output cannot be written, the reason goes here with exit 2, which blocks the call. |
| CLI | Results for the terminal | Errors prefixed `baley:` |

The guard's output is at `crates/baley/src/guard_hook/render.rs:46-99` and `guard_hook/mod.rs:168-197`.

## What it executes

All of the binary's child processes start at one place, `Command::new` at `crates/baley/src/process.rs:407`. No launch clears the environment: each child inherits Baley's environment, plus or minus the variables its call site sets.

| Program | Found by | When | Limits |
|---|---|---|---|
| `git` | `PATH` | Settings, checkout facts, the guard's branch, owner name, anchors | 60 s deadline (`crates/baley/src/git_process.rs:8`). The guard's calls get at most 5 s in total, in their own process group (`crates/baley/src/guard_budget.rs`). Forge calls set `GIT_TERMINAL_PROMPT=0` and cap output at 16 MiB. Local reads set `GIT_NO_LAZY_FETCH=1`. The branch read unsets `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR` and `GIT_NAMESPACE`. |
| `claude` | `PATH` | `baley install`: `mcp add-json` and `mcp remove` | 30 s, 64 KiB output |
| `~/.local/bin/baley update detached` | Absolute path | `baley serve` at startup, only when the global `updates.auto = true`, at most once a day | No timeout, output discarded (`crates/baley/src/update/detached.rs:9-26`) |
| `<versions>/<v>/baley update seed` | Absolute path | After an update stages a new binary | 60 s, 4 KiB output (`crates/baley/src/update/seed.rs:35-44`) |

No reachable code runs a shell. Baley passes no `-c` overrides or `GIT_CONFIG_*` variables to git, so every git call honours the repository's own `.git/config`. That config can name programs: `core.sshCommand`, credential helpers, `core.fsmonitor` and protocol allow-lists. The anchor push passes `--no-verify`, which skips the `pre-push` hook.

**Hooks installed into Claude Code.** One `PreToolUse` entry (`crates/baley/src/host_artifacts/hook.rs:13-49`):

- Matcher: `Bash|Monitor|PowerShell|Read|Grep|Glob|Write|Edit|NotebookEdit`.
- Command: the stable executable path, POSIX single-quoted, then `guard`.
- Timeout: 10 seconds.

The guard launches no program except git (design 0010 GRD-R14). MCP tool calls (`mcp__baley__*`) are not in the matcher.

**MCP registration.** `{"command": "<absolute executable>", "args": ["serve"]}`, with no environment and no working directory (`crates/baley/src/host_artifacts/registration.rs:20-27`). The server finds the project from `CLAUDE_PROJECT_DIR`.

**Commands, SQL and prompts built from data.**

- **Git arguments.**
  - The anchor push URL is the single output of `git remote get-url --push --all <git.remote>`, where `git.remote` comes from the committed `baley.toml`. It is passed to `push`, `ls-remote` and `fetch` without a `--` separator (`crates/baley/src/ledger/forge.rs:166-199`).
  - The remote name must appear in `git remote` output (`forge.rs:59-71`).
  - Tag names are built from the project id and a sequence number.
- **SQL.** rusqlite with bundled SQLite, and values are bound as parameters. `format!` SQL inserts only identifiers: compiled view names checked by `safe_identifier` (`crates/baley-store-sqlite/src/view.rs:836-846`), registry table names, and constant PRAGMAs. The `main` or `source` schema name in `view.rs:326` is guarded only by a `debug_assert`. No FTS5. `[VERIFY]` that every query was covered; the trace did not read each one.
- **Regex.** None is built from input in reachable code. The MCP `document-search` operation that took a pattern is retired.
- **Prompts.** Instruction text is compiled into the binary and served by the `instruction` operation, and no prompt is built from read data. The `document` operation returns stored capture text and scope drafts, written earlier by agents, into the model's context unchanged.

## What it must never touch

- **API keys and providers.** It never reads, stores or sends API keys, and never calls a model provider. Keys stay in the owner's environment, and the owner's session sends any outside request.
- **Baley's home and config folders.** An agent must never read or write them. The sandbox, the `Read` and `Edit` deny rules, and the guard's refusal of `Read`, `Grep`, `Glob`, `Write`, `Edit` and `NotebookEdit` calls into them carry this (design 0010 GRD-R11, GRD-R13).
- **Placed artifacts.** An agent must never write the placed stubs, the Claude Code settings and registration files, the installed executable or the versions folder.
- **Owner acts.** An agent must never get an act recorded as the owner's.
  - Approvals are served on the command line only, never over MCP, and `baley_apply` refuses `scope-approve` (`crates/baley/src/mcp/handler.rs:535-547`; ADR 0044).
  - Over MCP, only `capture` and `scope-submit` write to the ledger.
- **Data leaving the machine.** Data must never leave except in two ways:
  - anchor tags pushed to the push URL of `git.remote` when the owner runs `baley anchor`;
  - downloads from `updates.source`.
  - There is no telemetry.
- **Git on the owner's behalf.** It never commits, pushes anything but anchor tags, changes git config, or creates local refs.
- **Sandbox settings.** Install must never write settings that weaken the sandbox. It refuses a conflicting document instead (`crates/baley/src/host_artifacts/compose.rs:217-290`):
  - `sandbox.enabled` or `sandbox.failIfUnavailable` set to anything but `true`, or `sandbox.allowUnsandboxedCommands` set to anything but `false`;
  - `disableAllHooks` or `sandbox.filesystem.disabled` set to `true`;
  - another guard hook or another registration under Baley's key;
  - an `allowRead` or `allowWrite` entry that reopens Baley's folders or cannot be judged;
  - any `sandbox.excludedCommands` entry.
- **Privileges.** It never runs with elevated privileges. Everything it writes is under `HOME`, and `install.sh` uses no `sudo`.

## Who runs it and with what privileges

Everything runs as the interactive user. There is no service user, no root, and no timer or login item.

| Process | Started by | Inside Claude Code's sandbox? |
|---|---|---|
| `baley serve` | Claude Code, once per session, as a user-scope stdio MCP server | No. Hooks and local MCP servers run outside it (`security.rs:36-38`), with Claude Code's environment. |
| `baley guard` | Claude Code, before each call of the nine matched tools | No |
| `baley <command>` | The owner at a terminal | No |
| `baley <command>` run by an agent through `Bash` or `Monitor` | The agent | Yes. `denyRead` and `denyWrite` on Baley's home stop it from opening the ledger. |
| `baley update detached` | `baley serve`, when `updates.auto = true` | No |
| `install.sh` | The owner, by hand | No. It ends by running the downloaded binary as `baley install`. |

Nothing in the code tells the owner at a terminal apart from an agent running the same command. The sandbox is the only thing that separates them. `baley scope approve` records the actor `Owner` under the name in `owner.name`, or git's `user.name` when that is unset, read in the current directory so a repository-local value applies (`crates/baley/src/providers/identity.rs`, `crates/baley/src/scope/command.rs:240-337`). That name is a label, not authentication.

The control everything rests on is therefore the sandbox block and deny rules `baley install` writes. If they are missing or weakened, an agent's `baley scope approve` counts as the owner's. These belong in scope as critical:

- that install writes them correctly;
- that install refuses a document that would weaken them;
- that `baley doctor` reports drift and what an agent can reach (`crates/baley/src/host_doctor/`).

The user can already do everything Baley does. A real escalation is either:

- code confined by the sandbox reaching owner powers or code running outside the sandbox;
- an outside party getting code run as the user.

## What the worst outcome looks like

1. **An agent gets an owner act recorded,** such as approving its own scope draft (and, in later builds, truths and plans). Ways it could happen:
   - sandbox settings that install wrote wrong, that someone weakened, or that the doctor misses;
   - sandboxed code reaching the ledger;
   - an MCP operation that writes an owner act.
2. **Code runs outside the sandbox on the agent's behalf.** The guard, the server and the owner's own commands all run git outside the sandbox, in the agent's working tree, honouring that repository's `.git/config`. An agent that can write `.git/config` or `.git/hooks` from inside the sandbox can name programs that git then runs with the user's full rights. One example is a `core.sshCommand`, which the owner's `baley anchor` would run. `[VERIFY]` whether Claude Code's sandbox write-protects `.git/config` and `.git/hooks`.
3. **An agent commits to a protected branch, or pushes, without the owner deciding,** through a guard bypass within the shapes the guard claims to recognise, or by changing the policy at HEAD.
4. **The ledger is rewritten without detection.** The chain is unkeyed SHA-256 (`crates/baley-store/src/chain.rs:4-45`), so anyone who can write `baley.db` can rewrite it consistently. Only an anchor on the forge shows the rewrite, and anchors are pushed only when the owner runs `baley anchor` (Build 6, [#27](https://github.com/crenshawdev/baley/issues/27), automates it).
5. **A malicious binary is installed by an update.** Whoever controls `updates.source`, or TLS to it, controls the binary, since the manifest is unsigned ([#14](https://github.com/crenshawdev/baley/issues/14)). It then runs as the user in every new session.
6. **A secret leaks.** A secret pasted into capture text or a scope draft is stored raw in the ledger and returned over MCP into the model's context.

What the owner would not notice:

- a guard pass, which prints nothing;
- a declined command, which passes with nothing recorded;
- a pass on failure, whose stderr line Claude Code sends only to its debug log;
- a detached update, whose output is discarded;
- a ledger rewrite, until `baley verify` or `baley doctor` runs against an anchor.

## Trust boundaries (where untrusted data enters)

### 1. MCP over stdio

`baley serve` uses its own transport (`crates/baley/src/mcp/serve.rs:187`). There is no HTTP listener.

- **Frames.** One JSON line per frame, at most 4 MiB (`MAX_FRAME_BYTES`, `crates/baley/src/mcp/frame.rs:15`) and 128 levels deep. Each frame must be valid UTF-8 and pass a syntax check. A bad frame is answered and reading continues. An id over 256 bytes is answered with null.
- **Queue.** One call running and four waiting, with 16 MiB of frame bytes in total (`crates/baley/src/mcp/queue.rs:16-20`).
- **Client.** Its identity is self-reported, and only the `claude-code` host is accepted (`crates/baley/src/mcp/client.rs:1-5`, `gate.rs:79-87`). The identity is not authorization.
- **Tools.** Three tools: `baley_version`, `baley_query` and `baley_apply` (`crates/baley/src/mcp/tools.rs:97-119`).
  - Query serves `help`, `schema`, `instruction` and `document`.
  - Apply serves `capture` and `scope-submit`.
  - Every other operation name answers retired (`crates/baley/src/mcp/operations.rs:88-199`).
- **Arguments.** Each operation parses with serde and refuses unknown fields. `request_id` must be a lowercase hyphenated UUID, and capture text must not be blank. Error echoes are capped at 128 bytes.
  - Capture text and scope-submit fields have no length or count limit beyond the frame. Capture text over 4,096 bytes is stored as a payload.
  - No argument supplies a path or a working directory.
- **Writes.** `capture` and `scope-submit` first record `checkout.seen` and `policy.effective`, and need the project already in the ledger (`crates/baley/src/mcp/prepare.rs:340-356, 639-679`). Write preparation runs git in the project with no output cap (`process.rs:109`).

### 2. The guard hook's standard input

- **Input.** Read up to 64 KiB plus one byte. The guard reads `tool_name`, `cwd`, `session_id`, `tool_use_id` and `hook_event_name`, plus `command`, `file_path`, `notebook_path`, `path`, `glob` and `pattern` from `tool_input` (`crates/baley/src/hook_input/mod.rs:281-318`). It never reads `transcript_path`.
- **Fields.** `tool_name` is taken as given. A missing `hook_event_name` is accepted. `cwd` must be absolute and is the base for commit targets and path checks.
- **Project.** The project comes from `CLAUDE_PROJECT_DIR` and a `baley.toml` above it, never from `cwd` (`crates/baley/src/guard_hook/context.rs:65-174`).
- **Commands.** The scan of a `Bash` or `Monitor` command is `crates/baley-core/src/guard/scan.rs`. Its stated limits are listed below.
- **Path tools.** Their targets are canonicalized, following links, compared by component and by (device, inode) identity, and denied when they cannot be resolved (`crates/baley/src/protected_paths/`).

### 3. Command-line arguments and standard input

Parsed by clap.

- `baley scope approve` takes a digest of exactly 64 lowercase hex characters (`scope/command.rs:298-306`).
- `purge` validates its hashes and reason (`crates/baley/src/ledger/args.rs:68-80`).
- `models import` checks its trailer strictly. Model ids only have to be non-empty (`crates/baley-core/src/catalog/detection/parse.rs:38-51`).

### 4. Environment variables

Listed under What this tool reads. A relative `HOME`, or one containing `..`, is refused for the update layout (`crates/baley/src/update/installation.rs:80-107`).

### 5. Settings files

Parsed with the `toml` crate (`crates/baley-core/src/policy/parse.rs:509-527`). `[VERIFY]` whether it limits nesting depth.

- **Global only:** `updates.auto`, `updates.source` (must be `https://` with no query or fragment) and `owner.name`.
- **Project only:** `git.remote`, `git.protected_branches` (default `main` and `master`), `git.on_protected` (`ask`, `refuse` or `allow`, default `ask`) and `git.guard_hard_fail`.
- **Either file:** role models and effort, `escalate_on_failure`, and `review.reviewers`.

The schema is at `crates/baley-core/src/policy/schema.rs:197-385`. Scope is enforced at `parse.rs:776-783`.

- **The global `config.toml`** is the owner's. Agents cannot read or write it through the sandbox.
- **The project's `baley.toml`** is committed, so it is whatever the repository says.
  - The guard binds a project on any `baley.toml` with no ledger check, and takes the policy from HEAD's copy. A cloned repository's committed `git.on_protected = "allow"` or `protected_branches = []` turns commit gating off there. Push still asks.
  - The guard denies `Write`, `Edit` and `NotebookEdit` on the session project's `baley.toml`. The sandbox's `denyWrite` does not list it, because install is user-level and knows no project, so sandboxed `Bash` can edit the working-tree copy. A changed policy takes effect only once it is committed at HEAD. `[VERIFY]` that this is intended.

### 6. Git output and repository-local git config

Git's stdout and stderr are parsed: branch names, object ids, remote URLs and HEAD's `baley.toml`. Remote URLs have userinfo stripped before `checkout.seen` records them, but query strings keep their contents (`crates/baley/src/checkout/strip.rs:13-77`). Git's stderr has control characters and userinfo removed before it is printed or recorded. Repository-local config is agent-writable from inside the sandbox unless Claude Code protects it (see worst outcome 2).

### 7. Claude Code's settings files

Read by install, the doctor and the interview with no size cap, following links for reads and refusing links for writes. Each must be a JSON object.

### 8. The update source

`crates/baley/src/update/fetch.rs`:

- **Client.** `https_only(true)`, rustls with bundled webpki roots (not the system store), no proxy, no referer, and the Authorization header that URL userinfo would add is removed.
- **Limits.** 20 s to connect. The manifest gets 64 KiB in 20 s, and the binary 256 MiB in 10 min.
- **Manifest.** Exactly two lines (`crates/baley/src/update/manifest.rs:100-135`). Only a strictly newer version is taken.
- **Check.** The binary's SHA-256 must match the manifest, which comes from the same source and is not signed ([#14](https://github.com/crenshawdev/baley/issues/14)).
- **Redirects.** reqwest's default policy. `[VERIFY]` that `https_only` holds on every redirect hop.
- **install.sh.** Uses `curl -q --proto =https --tlsv1.2 -fsSL` with the same SHA-256 check (`install.sh:67-71, 142-171`).

### 9. Anchors read back from the forge

Read through git with a 60 s deadline and a 16 MiB output cap.

- `ls-remote` output is parsed strictly (`crates/baley/src/ledger/forge.rs:238-286`).
- The tag message's first line must match the canonical JSON byte for byte, and its sequence must match the tag name (`crates/baley-core/src/forge.rs:84-123`).
- Tag signatures are ignored, and transport security is whatever git uses for that remote.

### 10. Trusted only because Baley wrote it earlier

- **The ledger.** Opening it runs only `PRAGMA quick_check` (`crates/baley-store-sqlite/src/health.rs:22-38`).
  - Full chain verification runs only in `verify`, `doctor` and `anchor` (`crates/baley-store-sqlite/src/ledger.rs:227-278`).
  - Appends extend the head stored in the project table (`crates/baley-store-sqlite/src/transact.rs:518-524`).
  - Replays on MCP and in the guard return the stored answer without judging again.
  - Views are read as stored.
  - Payloads are opened by hash without re-hashing, and zstd decompression has no output limit (`crates/baley-store-sqlite/src/payload.rs:96-121, 317-320`).
- **The guard's remembered policy** can only add denials (`crates/baley-core/src/guard/answer.rs:104-131`).
- **A staged binary** is reused when its hash matches.
- **The install record** in the ledger drives the install plan.

### Stated limits of the guard, not findings

Design 0010 states these. They are not new findings:

- **GRD-R3: only commit and push.**
  - Only `git commit` and `git push` in a `Bash` or `Monitor` command are scanned, and no other git verb is covered.
  - A segment counts only when its first word is `git` or ends in `/git`, so git aliases and wrappers such as `env`, `sudo`, `sh -c` or `X=1 git` are not recognised.
  - The scan declines substitutions, backticks, redirects, subshells, braces, a leading comment, a NUL, an unclosed quote and a trailing backslash, and a declined command passes with nothing recorded.
  - A command that moves a commit by a means outside the recognised shapes is judged as though it ran from the hook's working directory.
  - A `PowerShell` call in a project is asked about, never scanned.
- **GRD-R2: outside a project.** The command guard is silent for `Bash`, `Monitor` and `PowerShell`.
- **GRD-R14: input size and the time bound.**
  - Command-tool input over 64 KiB, or input that cannot be parsed, passes. The six path tools are denied instead.
  - The check of the `user` chain on each guard write is unbounded, so the 10-second answer is not guaranteed on a long chain. Build 9 bounds it.
- **GRD-R6: failed git reads.** A failed git read for a detected commit passes, with a stderr line and a recorded guard failure, unless `git.guard_hard_fail` is set and the branch is provably protected.
- **GRD-R13: Bash file access and keys.**
  - The guard does not check `Bash` file access. The sandbox carries it.
  - The folder denials do not protect API keys in the owner's environment (CFG-R24, ADR 0039). Every child process inherits that environment.

### Known and tracked, not findings

- **[#14](https://github.com/crenshawdev/baley/issues/14):** release manifests are unsigned.
- **[#30](https://github.com/crenshawdev/baley/issues/30):** the parked review engine is still compiled in.
- **[#26](https://github.com/crenshawdev/baley/issues/26):** a plan's file lease is not enforced when a task closes.
- **[#27](https://github.com/crenshawdev/baley/issues/27):** anchors are pushed only by hand.
- **[#24](https://github.com/crenshawdev/baley/issues/24):** macOS is written but not run or qualified, and CI runs on Linux only.

## Secrets that may pass through

**Secrets that can appear in the data Baley handles:**

- API keys or tokens an agent pastes into capture text or a scope draft;
- credentials in git remote URLs, as userinfo or query strings;
- credentials in git's stderr;
- credentials in the `updates.source` URL.

**Where they can end up:**

- **The ledger and the model's context.** Capture text and scope drafts are stored unredacted in the ledger and returned unredacted by the MCP `document` operation. The remedy is `baley purge`, which reminds the owner to rotate the secret (`crates/baley/src/ledger/display.rs:20`), with `secure_delete = ON`.
- **Remote URLs.** They are recorded with userinfo stripped. A secret in a query string is kept.
- **Process arguments.** The anchor push URL, including any `user:token@`, is passed in git's arguments and is visible in the process list. Only the timeout text is redacted (`crates/baley/src/ledger/forge.rs:167-175, 872-892`).
- **The terminal.** An update failure prints the `updates.source` address in full, including any userinfo (`crates/baley/src/update/command.rs:277`). Recorded update events hold codes only.
- **Guard records.** They keep the digest of the input fields the guard read, never the command text, and git's stderr excerpt is replaced by `[redacted]` (`crates/baley/src/guard_hook/record.rs:33, 356-366`).
- **Child processes.** Every child (git, claude, the detached update) inherits the full environment, including any API keys in it. Nothing scrubs a key that a command prints.

**Secrets the tool itself holds.** None. It holds no signing key, token or keyring, and anchor tags are unsigned. Anchor pushes use the owner's own git credentials (credential helper, SSH agent or the inherited environment) with prompts disabled. File modes: the home and config folders are 0700, and `baley.db`, its lock files, exports and `install.lock` are 0600. `config.toml` and `baley.toml` keep their existing mode, or get the umask default when new.
