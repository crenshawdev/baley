# 0010: Guard

| | |
|---|---|
| Status | Accepted |
| Design issue | none; build issue [#24](https://github.com/crenshawdev/baley/issues/24) |
| Requirement prefix | GRD |
| Applies | [0002: System design](0002-system-design.md) |
| Related | ADRs: [0008](../adr/0008-host-sandbox-isolation.md), [0009](../adr/0009-served-instructions.md), [0027](../adr/0027-vendor-folders-and-plain-keys.md), [0033](../adr/0033-host-security-bar.md), [0036](../adr/0036-per-user-guard-records.md), [0039](../adr/0039-session-owned-provider-credentials.md), [0040](../adr/0040-guard-commit-checkout.md) · C4 view: components ([0002](0002-system-design.md) Figure 4) |

The current design of this area, and nothing else. Edit it in place when the design changes; git holds the history. It describes the design only, never the work still to do.

## 1. Purpose and scope

This area decides what Baley does at the host's edge, before a tool call an agent makes runs:

- the one hook Baley installs, what it sees on Claude Code, and how it finds the project;
- the command guard, for `Bash`, `Monitor` and `PowerShell` commands: which git commands it acts on, what it asks, what it refuses, and what it does when it cannot decide;
- the file guard, for `Write`, `Edit` and `NotebookEdit`: which paths an agent may not write;
- how each answer is recorded, remembered and replayed;
- the answer adapter that renders an answer in the host's hook form, and the protection that keeps agents from reading or writing Baley's home and its config folder.

It does not decide the lease itself or what an out-of-lease commit does at task close ([0006](0006-execution.md)); which branch work happens on or how landing pushes ([0011](0011-milestones-landing-undo-pause.md)); risk detection ([0009](0009-risk.md)); or how stubs are rendered and installed ([0012](0012-host-interface.md)).

Hand-offs: 0003 gives the guard project discovery and the settings it reads; 0006 gives it the active lease; 0012 installs the hook and the sandbox configuration; the ledger (0001) stores every guard record, in the per-user project `user`.

In the component view of [0002](0002-system-design.md) (Figure 4) the guard is the third way into the host interface, beside the MCP server and the command line.

## 2. Terms

| Term | Meaning |
|---|---|
| Hook | Claude Code's pre-tool-use call into Baley, for `Bash`, `Monitor`, `PowerShell`, `Read`, `Grep`, `Glob`, `Write`, `Edit` and `NotebookEdit`. The host passes the tool, its input, the working directory, the session and a call id, and sets `CLAUDE_PROJECT_DIR` in the hook's environment. |
| Guard | Baley's answer to a hook call: `pass`, `ask`, `deny`, or `pass on failure`. |
| Verb | The git subcommand a `Bash` or `Monitor` command carries: `commit` or `push`. A `PowerShell` command is not scanned. |
| Protected branch | A branch named in `git.protected_branches`. |
| Torn settings | A settings file that cannot be read or parsed at the moment of the call, or that holds a value outside its type or grammar (CFG-R9), such as a bare-string branch list or an unknown `git.on_protected`. |
| Guard failure | A call the guard could not decide because git or the branch could not be read. |
| Hard fail | The opt-in rule (`git.guard_hard_fail`) that turns a guard failure on a provably protected branch into a deny. |
| Remembered policy | The denials of the last complete policy the guard recorded beside an answer, kept per session project root, target checkout root and host so that a denial can still be given when the settings are torn: whether `git.on_protected` was `refuse`, whether hard fail was on, and the protected list when either was. A complete policy read for a call that passes records nothing, so it leaves the remembered denials as they were. It never supplies an allow. |
| Redelivery | The host calling the hook again with the same call id after a timeout. |
| Answer adapter | The renderer that turns a guard answer into Claude Code's pre-tool hook output: an `ask` or `deny` decision with its reason on stdout, nothing for a pass, and one stderr line for a pass on failure. It never answers `allow`. |
| Unrecordable | A decision the guard cannot record before it answers (GRD-R9). Nothing is appended for it. |
| Sandbox | The host's own restriction on what an agent process may read and write ([ADR 0008](../adr/0008-host-sandbox-isolation.md)). |

## 3. Requirements

| Id | Rule | Why | Depends on | Status |
|---|---|---|---|---|
| GRD-R1 | Baley installs one hook: Claude Code's pre-tool-use call for `Bash`, `Monitor`, `PowerShell`, `Read`, `Grep`, `Glob`, `Write`, `Edit` and `NotebookEdit`, with a bounded timeout. There is no other hook. | One edge for every tool that runs a shell command or reads or writes a file. | SYS-P11, SYS-P12 | Active |
| GRD-R2 | The guard takes the project from the hook's `CLAUDE_PROJECT_DIR`, walking up from it to the nearest `baley.toml` and stopping at the git repository root (CFG-R4). Only not-found is absence for `baley.toml` and `.git`. A `baley.toml` entry, including a directory or dangling link, or a metadata error other than not-found binds that folder as the project. A `.git` entry or a metadata error other than not-found makes the folder the repository root and ends the walk. A `CLAUDE_PROJECT_DIR` that is missing, empty, not UTF-8, relative, over 4,096 bytes or not a directory, or one in no project, means no project: the working directory never stands in for it, and no project is invented for the record. The hook's working directory is the base for tool targets. For the shapes GRD-R3 recognises, a commit is judged at the working directory or the directory reached by applying each `-C` operand in order, or asks when its target is unestablished. Neither a `/cd` nor a redirected commit changes the session project or whose policy applies. Outside a project the guard is silent for `Bash`, `Monitor` and `PowerShell` calls and records nothing. The path rules need no project and apply either way: the `Write`, `Edit` and `NotebookEdit` refusal (GRD-R11) and the refusal of a `Read`, `Grep` or `Glob` call that reaches Baley's home or its config folder (GRD-R13). | The guard acts only where Baley is responsible. | CFG-R4 | Active |
| GRD-R3 | The command guard reads the command of a `Bash` call and of a `Monitor` command watch with one scan, and acts on a command only when it carries a git `commit` or `push` verb. It splits the command on `;`, `|`, `&`, `&&`, `||` and newlines, takes segments whose first word is `git` or ends in `/git`, reads the commit target before the verb while skipping recognised git global flags and their operands, and declines to judge a command containing substitutions, backticks, redirects, subshells, braces, a leading comment, a NUL, an unclosed quote or a trailing backslash. A commit carries `Cwd` when nothing redirects it, or `Directory(operands)` for one or more `-C` operands kept in order, reading the attached `-Cpath` form as a redirect out of caution although git rejects it. A `-C` after `commit` reuses a commit message and does not change the target. `--work-tree` and `-c` do not redirect the checkout. The target is `Unestablished` for `--git-dir` in either form, an empty `-C` operand or one starting with `~` or containing `*`, `?` or `[`, or commit segments naming different targets. An earlier segment also makes it `Unestablished` when, after dropping leading `NAME=value` words and then leading `builtin` or `command` words, its first word is `cd`, `pushd`, `popd`, `chdir`, `export`, `eval`, `source`, `.`, `declare` or `typeset`; when the segment consists only of `NAME=value` words; or when the remaining first word is `set` and an argument is a single-hyphen flag cluster containing `a` or the word `allexport`. An assignment name starts with an ASCII letter or underscore and continues with ASCII letters, digits or underscores. These checks match segment words, never text elsewhere in the command. `set -e`, `set -euo pipefail`, `command -v git`, `echo cd /r` and `git add .` leave a later commit at `Cwd`. The guard does not work out where a `cd` or a sourced file leads, so a commit after one asks in a bound project even when the directory does not change. These are the shapes the guard recognises: a command that moves a detected commit by a means outside the list is judged as though it ran from the hook cwd. Removing prefixes for this check does not make a segment whose first word is not `git` or a path ending in `/git` into a detected commit. Repeated commits naming the same target keep it. A command that carries both verbs is judged as a push. A declined command passes with nothing recorded, and so does a `Monitor` watch with no command, which is not scanned. In a bound project a `PowerShell` call asks, whether or not it mentions git, and the reason names the grammar the scan cannot judge. A `PowerShell` call is never scanned. No other git verb is covered; this is a stated limit of the design. | Commit and push are where work reaches the record and the forge; everything else was tried and did not pay. Baley reads POSIX shell, so it asks about PowerShell instead of guessing. | | Active |
| GRD-R4 | A `push` always asks, on any branch, with a fixed reason. | Publishing is the owner's step. | SYS-P5 | Active |
| GRD-R5 | A `commit` whose target is `Unestablished` asks with the fixed reason that the guard cannot tell which checkout it lands in, without reading a branch or policy. Otherwise a `commit` on a protected branch follows `git.on_protected`: `ask`, `refuse` (deny) or `allow`, acting on the branch git read at the commit's target directory. An unknown value is outside the setting's grammar, so the file is torn (CFG-R9) and the commit asks, naming the file, unless a remembered denial applies (GRD-R7). On any other branch a commit passes. | The owner sets the branch discipline once. | CFG-R5, CFG-R9 | Active |
| GRD-R6 | For a detected commit whose target GRD-R3 establishes, when git or the branch at that target cannot be read, the guard passes, prints a loud line on stderr, and records a guard failure. When `git.guard_hard_fail` is set and the branch is provably protected from what could be read, it denies instead. This rule handles failures of those reads, not command shapes outside GRD-R3: a command that moves a detected commit by a means outside that list is judged as though it ran from the hook cwd, and a declined or undetected command passes with nothing recorded. | A guard that cannot decide must not silently become a wall or a hole. Commands outside GRD-R3's recognised shapes are a stated limit of the scan. | | Active |
| GRD-R7 | When a settings file is torn, the guard asks, naming the file. A nearest `baley.toml` that is not a regular file or cannot be read is torn, and a dangling link is named as a link to a missing file. None hands the project to an outer file. Two denials from the remembered policy still stand: a remembered `refuse` on a branch git read that is in the remembered protected list, and a remembered hard fail on a provably protected branch when git cannot read the branch. A remembered `allow` or `ask` never relaxes the ask. The remembered policy keeps only denials, never an allow. | Torn settings must not open the door. | CFG-R9 | Active |
| GRD-R8 | Every `ask`, `deny` and guard failure is recorded before it is answered, for every tool: a path tool's denial and a `PowerShell` ask are recorded as a commit's answer is. The record holds the host, the session and the call id; `CLAUDE_PROJECT_DIR` as given and the working directory, as separate facts; the tool and the digest of the input fields the guard read, never the command; the nullable `target` as text, holding a path tool's resolved target or a redirected commit's resolved directory; the verb, the branch, the settings in force, the outcome and the reason. When HEAD's copy of `baley.toml` is torn, the record keeps Baley's words for it with git's stderr excerpt replaced by `[redacted]`, while the live answer keeps the bounded excerpt. Records go to the per-user project `user` at policy version 0, so recording needs no project in this machine's ledger and admits no checkout. A plain pass, a command the scan declines and a `Monitor` watch record nothing, and input the guard cannot read is denied with nothing recorded, since it carries no call to record it by. | The record shows what the guard held and why, and keeps no command text and no output of git beyond Baley's own words. | SYS-P6 | Active |
| GRD-R9 | When the decision cannot be recorded, an `ask` becomes `deny` with the reason that the guard could not record its decision, and a loud stderr line reports the recording failure. A decision is unrecordable when a store read or write fails, a guard store that stays busy past its storage time included; when `user`'s views need a rebuild, and then the line names `baley rebuild user`; when the call has no envelope or call id, or caller text (call id, working directory, `CLAUDE_PROJECT_DIR` or session) cannot be recorded as given; when its call id was already recorded for another input digest, project directory or working directory; when a redirected commit's checkout root cannot be read, was not walked or is not UTF-8; when the session project root or cwd checkout root is not UTF-8; when settings have no bound session project or torn settings lack a branch or git verb; when the transaction returns successfully without staging or replaying an answer; or when Baley's home folder cannot be resolved. When the transaction returns successfully without staging or replaying an answer, the stderr line reports a call-id clash: the call id was answered before for another input, project directory or cwd. Nothing is appended for an unrecordable decision, so one call id never has two records. A `deny` stays a deny, a pass on failure stays a loud pass and a plain pass stays a pass. A decision that was recorded is not changed, so a recorded ask about torn settings stays an ask. | An unrecorded ask is the gap the record exists to close. | GRD-R8 | Active |
| GRD-R10 | A call whose host, session and call id match a recorded answer, with the same input digest, project directory and working directory, gets that answer again from the record, even after the policy changed. A command with a commit or push verb, or a `PowerShell` call, in a project is looked up before any git or policy read, and the transaction that would record a call looks it up again before it appends, so two deliveries racing each other get one record and one answer. | The host may deliver a call twice; the answer must not differ. | EVD-R26 | Active |
| GRD-R11 | The file guard denies a `Write`, `Edit` or `NotebookEdit` call whose target is inside or is one of these protected paths: Baley's home folder; Baley's config folder (the global settings file `config.toml`, [0003](0003-configuration-and-routing.md) CFG-R2); the `baley.toml` of the session's project and the `baley.toml` of the checkout the hook's working directory is in, by path or by file identity, but no other file of that name; the stub paths Baley supplies; the Claude Code settings and registration files Baley places; the supplied installed executable path; and the staged-version folder once T14 defines it. The placement projection supplies the stub paths, the placed settings and registration files and the installed executable path, and T15 passes that list to the guard; T14 adds the staged-version folder. It judges them from any working directory, with or without a project. Path resolution canonicalizes the existing prefix, reads both the spelling given and the spelling with each backslash taken as a slash, and refuses an empty value, control bytes, a leading `//` or drive-letter prefix, and a non-directory parent. It judges containment by component and by the (device, inode) identity of existing ancestors, so a sibling such as `baley-old` is not inside `baley` and a differently cased spelling on a case-insensitive volume is still caught. A path it cannot resolve is denied. During an active dispatch, a path outside the dispatch's lease is denied (EXE-R8). Build 5 owns the lease: the decision takes it as an input whose only value today is no active dispatch, and Build 5 defines what a lease names and the deny for a write outside it. | The owner sets policy, Baley renders stubs, and the lease means something as the write happens. | CFG-R11, ADR 0009, EXE-R8 | Active |
| GRD-R12 | The answer adapter renders each answer in Claude Code's pre-tool hook form. An `ask` or `deny` is a `hookSpecificOutput` object with `hookEventName` `PreToolUse`, `permissionDecision` `ask` or `deny`, and the reason as `permissionDecisionReason`, cut to 10,000 bytes on a character boundary, on stdout with exit 0. A plain pass prints nothing and exits 0, so the host's own permission rules decide. It is never `allow`, which would skip them. A pass on failure prints its reason as one stderr line and exits 0. When an `ask` or `deny` cannot be written to stdout, the guard exits 2 with the reason on stderr, which blocks the call. | One guard decision, rendered in the host's form, and a pass never widens what the host allows. | SYS-P8 | Active |
| GRD-R13 | Agents can neither read nor write Baley's home or its config folder. Three mechanisms carry it, configured at install: Claude Code's sandbox, for shell commands and their children (`Bash`, `Monitor`, `PowerShell`); its `Read` and `Edit` deny rules, for the built-in file tools; and the guard's refusal of a `Read`, `Grep` or `Glob` call that reaches either folder, because Claude Code applies `Read` rules to `Grep` and `Glob` only on a best-effort basis. The guard takes the call's target from `Read`'s `file_path` or from `Grep`'s or `Glob`'s `path`, which is the hook's working directory when absent. It also takes the folders a `Grep` `glob` or a `Glob` `pattern` names before its first wildcard, joined to that target. It refuses when a target lies inside either folder or holds one, compared by component and by file identity, so a sibling such as `baley-old` is not matched. It refuses a pattern with a `..` component after a wildcard, and a path it cannot resolve. An unavailable sandbox is reported, never passed over. API keys stay in the owner's environment, which these folder denials do not protect (CFG-R24, ADR 0039). `baley doctor` checks the configuration and reports what an agent can reach, reads included. | The record and global settings are protected by the host and the guard together, and tampering is detected by the chain and its anchors. | ADR 0008, ADR 0033 | Active |
| GRD-R14 | The guard reads at most 64 KiB of the hook's input and answers within the hook's 10-second timeout, on one budget that starts when the guard starts: 8 seconds of work; git at most 5 seconds in total, across the branch lookup and the up to three launches that read HEAD's copy of `baley.toml`; storage waits at most 2 seconds; and at least 1 second kept after the work for ending and reaping a child and writing the answer. Every limit shrinks with the work already done, and none is reset for a later step. The guard launches no program except git, for the branch and for HEAD's copy of the settings. When git gives no branch, because it cannot run, fails or has no time left, the guard reads the branch from `.git/HEAD` directly, bounded and without following symbolic links. It collects no remote and no root commit. Input it cannot read, or that is over the bound, is denied for `Read`, `Grep`, `Glob`, `Write`, `Edit` and `NotebookEdit`, and passes for the command tools. Standard input that fails to read at all is denied, since the tool is then unknown. | The guard must answer fast and must not become a way to run things. | | Active |

## 4. Roles and actors

| Actor | Receives | Returns | Model and effort from |
|---|---|---|---|
| Host | The guard's answer in its own form | Runs, asks the owner, or blocks the tool call | Not applicable |
| Owner | An `ask` put by the host | Yes or no, in the host | Not applicable |
| Agent (any worker or the session) | A denied or held tool call with its reason | Nothing; it does not argue with the guard | Not applicable |
| Baley guard | The hook input | pass, ask, deny, pass on failure | Not applicable |
| Ledger | Guard records, in the per-user project `user` | The answer recorded under a call id, and the denials remembered for torn settings | Not applicable |

No model is dispatched by this area.

## 5. Commands and operations

### baley guard (hook entry)

- **Inputs:** the hook's JSON on standard input, read up to 64 KiB. The guard reads these fields:
  - `tool_name`, the tool, and `tool_input`, its input;
  - `cwd`, the working directory;
  - `session_id`;
  - `tool_use_id`, the call id, which is never invented: a call with none cannot be recorded (GRD-R9);
  - `hook_event_name`, which must be `PreToolUse` when present.

  Each tool's `tool_input` is read as follows:

  | Tool | Fields read |
  |---|---|
  | `Bash` | `command` |
  | `Monitor`, command form | `command` |
  | `Monitor`, watch form | none: a watch has no `command` and is not scanned |
  | `PowerShell` | none: the call is judged by its tool name |
  | `Read` | `file_path` |
  | `Grep` | `path` and `glob`, both optional: an absent `path` means the hook's working directory. Grep's search `pattern` is not read |
  | `Glob` | `pattern`, required, and `path`: an absent `path` means the hook's working directory |
  | `Write`, `Edit` | `file_path` |
  | `NotebookEdit` | `notebook_path` |

  From its environment it reads `CLAUDE_PROJECT_DIR`, kept as given for the record.
- **Project and target:** the policy comes from the session project at `CLAUDE_PROJECT_DIR`, found by the walk of GRD-R2. The hook's working directory is the base that tool targets resolve against. For a commit, each `-C` operand is joined to the preceding directory, starting at that base, and an absolute operand replaces it. The branch and the bounded `.git/HEAD` fallback are read there, on the existing single branch launch and budget grant. A target the scan cannot establish asks without reading a branch or policy. A walk from the resolved commit directory supplies the canonical checkout root for remembered denials, never a policy. The separate walk from the hook's working directory still finds the `baley.toml` the file guard protects beside the session project's.
- **Protected paths:** the file guard protects Baley's home and config folders, the session project's `baley.toml` and the one that binds the working directory's checkout (`crates/baley/src/guard_hook/context.rs:106-114, 126-130`). Once T15 passes the placement projection in, the placed stubs, the placed Claude Code settings and registration files and the installed executable join that list (`crates/baley/src/host_artifacts/placement.rs:271-296`). T14 adds the staged-version folder.
- **Policy reads:** only a commit with an established target in a project reads a policy, after its branch. The guard reads the global `config.toml` and the working tree's `baley.toml`, each to at most 1 MiB, and HEAD's copy of `baley.toml` at the session project's root through the bounded git reader, and merges them with Claude Code's `[host.claude-code]` sections. A file over the cap, a working-tree file it cannot read or finds gone after the walk found it, and a config folder it cannot find each make the settings torn. It admits no checkout, runs no policy step and records no `policy.effective`.
- **Recording:** a guard record is one `guard.record` command in the per-user project `user`, on stream `guard`, at policy version 0, by `baley`, with the hook form of the caller.
  - A command with a `commit` or `push` verb, or a `PowerShell` call, in a project first looks up the answer recorded under its call id, on a short store open of its own before any git or policy read. A replay is answered from the record, with no git or policy read and nothing appended. With no `user` there is no record, and the lookup creates none.
  - An answer to record takes a second short open for the audit transaction, which creates `user` when it is missing.
  - Inside the transaction the call's `guard` document is read again before anything is appended, so a replay or a clash appends nothing.
  - Under torn settings the remembered denials are read there, and only there, and the commit is judged again with them. Under a complete policy, `guard.policy_recorded` is appended beside `guard.answered` when the denials changed.
  - A new answer counts as recorded only once the transaction commits.
- **Outputs:** per the renderer (GRD-R12). An ask or deny is a `hookSpecificOutput` decision on stdout with exit 0. A plain pass prints nothing and exits 0. A pass on failure prints its reason on stderr and exits 0. When an ask or deny cannot be written to stdout, the guard exits 2 with the reason on stderr, which blocks the call. Each unrecordable decision adds one loud stderr line naming its cause.
- **Refusals (as answers):**

  | Answer | When | Requirement |
  |---|---|---|
  | `ask` | push; commit whose target cannot be established; protected commit under `ask`; torn settings; a `PowerShell` call in a project | GRD-R3, GRD-R4, GRD-R5, GRD-R7 |
  | `deny` | protected commit under `refuse`; hard fail; remembered denial under torn settings; a Write, Edit or NotebookEdit to Baley's home or config folder, a protected `baley.toml`, a placed stub, Claude Code settings or registration file, the installed executable path or staged-version folder as GRD-R11 specifies, or, once Build 5 supplies the lease, outside it during a dispatch; a `Read`, `Grep` or `Glob` call whose target or pattern reaches either folder, with or without a project; malformed, oversized or incomplete input for one of the six path tools; standard input that cannot be read at all; a path call when Baley's home and config folders cannot be found; an ask that is unrecordable for any cause listed in GRD-R9 | GRD-R5, GRD-R6, GRD-R7, GRD-R9, GRD-R11, GRD-R13, GRD-R14 |
  | `pass on failure` | git or branch unreadable without hard fail | GRD-R6 |
  | `pass` | everything else, including a declined or unreadable command, a `Monitor` watch, and any `Bash`, `Monitor` or `PowerShell` call outside a project | GRD-R2, GRD-R3 |

- **Time (GRD-R14):** one budget per call, started before the input is read, inside Claude Code's 10-second hook timeout:
  - 8 seconds of work, ending 8 seconds after the start. The input read counts against it but is bounded by size, not time.
  - git at most 5 seconds in total. Each launch gets the smaller of git's time left and the work time left, and is charged the time it took. Once either is spent, nothing launches, since a zero timeout would still start git only to kill it.
  - storage waits at most 2 seconds in total, and never past the end of the work time.
  - at least 1 second after the work, for ending and reaping a child and writing the answer.
- **Programs:** git is the only program the guard starts: `symbolic-ref` for the branch, and `rev-parse`, `ls-tree` and `cat-file` for HEAD's copy of `baley.toml`, each in its own process group and killed at its timeout. The launch validator accepts a guard launch only at the timeout its budget grant gave it, above zero and at most 5 seconds. Every other caller keeps its exact registered deadline.
- **Storage:** the guard opens the ledger at most twice, for the lookup and for the audit transaction, each time on the storage time its budget has left and charged the time the open and its work took, so no open is held while git runs. Each open takes the writer queue and its connections by nonblocking tries until that time is spent, and runs each statement with SQLite's busy timeout set to the storage time left when the statement starts. A wait past the storage time answers busy. A project whose views are behind this binary's answers needs-rebuild, and the guard never rebuilds them and never takes the maintenance lock. A project with no events gets its first stamps in the guard's own bounded writer turn. Busy and needs-rebuild both leave the decision unrecordable (GRD-R9). Once a write transaction has begun, it runs to commit or rollback, and no deadline interrupts it.

### baley doctor (the guard's part)

- **Inputs:** the host name, the placement map's expected files and protected paths (`crates/baley/src/host_artifacts/placement.rs:231-296`), the settings and hook documents the map places, and the `user` project's view stamps from the store's own doctor. Until T15 supplies a placement, the command line gives a map whose every artifact has an unknown place (`crates/baley/src/host_doctor/mod.rs:69-89`), so the settings, hook, registration and stubs all read as not installed and nothing is judged against an invented document.
- **Outputs:** the host section the doctor prints after its ledger lines (`crates/baley/src/host_doctor/report.rs:26-148`). For the settings and hook documents it is given, it reports:
  - whether a `PreToolUse` item runs the guard for all nine tools of the hook's matcher, and whether `disableAllHooks` is true. The coverage judge credits `Bash`, `Monitor` and `PowerShell` to the sandbox and never asks about the guard for them, so this is a separate check (`crates/baley/src/host_doctor/protection.rs:97-161`).
  - which mechanism covers Baley's home and config folder for each tool and for each read or write, from the coverage judge (`crates/baley/src/host_doctor/protection.rs:48-92`, `crates/baley/src/host_doctor/report.rs:265-322`). Each verdict is worded as configuration of the named document, not as proof that Claude Code enforces it, and `Grep` and `Glob` are marked best-effort.
  - the programs the sandbox needs, looked up on the command's own `PATH`: `bwrap` and `socat` on Linux, none on macOS, and the sandbox unsupported on any other platform. A missing program marks the sandbox unsupported for the coverage judge (`crates/baley/src/host_doctor/prerequisites.rs:75-182`).
  - whether the `user` project's views were behind this binary's when the doctor started, naming `baley rebuild user` (`crates/baley/src/host_doctor/guard_records.rs:38-78`, `crates/baley/src/host_doctor/report.rs:220-233`). The store's doctor reads the stamps before it verifies the views, and verifying brings them current, so the line says what the guard would have found.
- **Exit status:** a gap in a given document, a missing sandbox program, an unsupported platform, a ledger at a newer epoch and views behind this binary's raise it to 1. An artifact that is not installed does not. No host finding gives 2 or 3.
- **Limits:** only the documents given are judged. Other Claude Code settings files and `sandbox.enabledPlatforms` can change what applies, and the report says so. The doctor carries no live evidence: how Claude Code honours each answer is the dated host-matrix record cited in section 11, not a doctor output. A stored view this binary no longer declares is not in the stamps, so it is not seen. WSL1, containers that block bubblewrap, Ubuntu's AppArmor rule on user namespaces, ripgrep and the seccomp filter are not checked.
- **Planned (T17):** the delivery checks over what `baley install` wrote, among them that the hook points at the stable path and that the installed settings hold the rendered content. They run through the same observe, judge and report steps over a placement map the installer supplies ([ADR 0038](../adr/0038-installer-and-opt-in-updates.md)).
- **Refusals:** none; findings are reported (GRD-R13).

## 6. Records

### guard.answered (event, per-user project `user`, `guard` stream)

One recorded answer: an ask, a deny or a pass on failure, version 1. It is appended by a `guard.record` command at policy version 0 by `baley`, whose caller is the hook form ([0012](0012-host-interface.md) section 6). The session project is a fact here and in the caller, never the ledger project, so recording needs no project in this machine's ledger.

| Field | Type | Meaning |
|---|---|---|
| `host` | text | `claude-code` |
| `session` | text or null | The host's own session id, null when the call had none |
| `call` | text | The host's call id, `tool_use_id` |
| `project_directory` | text or null | `CLAUDE_PROJECT_DIR` as the host gave it, null when unset |
| `cwd` | path | The hook's working directory |
| `tool` | `Bash`, `Monitor`, `PowerShell`, `Read`, `Grep`, `Glob`, `Write`, `Edit`, `NotebookEdit` | The tool's name as the host gives it |
| `input_digest` | hex SHA-256 | Of the canonical JSON of the tool's name and the input fields the guard read (section 5). Never the command text |
| `target` | path or null | A path tool's resolved target or a redirected commit's directory after applying its `-C` operands to `cwd`, as text. Null for a commit at `cwd`, an unestablished target, or a call with no resolved target. The directory retains path components such as `..`, so filesystem resolution preserves symlink semantics |
| `verb` | `commit`, `push` or null | |
| `branch` | name or null | The branch git read, or the one `.git/HEAD` named |
| `settings` | object or null | A complete policy's `protected_branches`, `on_protected` and `hard_fail`; or `torn`, naming the torn file in Baley's words with git's excerpt replaced by `[redacted]`; or null when no settings were read, as for a push, a commit with an unestablished target, a `PowerShell` ask or a path answer |
| `outcome` | `ask`, `deny`, `pass-on-failure` | |
| `reason` | text | The reason, with git's excerpt redacted as in `settings` |

### guard.policy_recorded (event, per-user project `user`, `guard` stream)

The denials of the last complete policy recorded for one session project, target checkout and host, kept for GRD-R7, version 1. It is appended only inside the transaction that appends a `guard.answered`, and only when the denials differ from those remembered under its key, so a newer complete policy with no denial clears an older `refuse` once a call under it is recorded. A plain pass under that policy records nothing and clears nothing. The target checkout root is discovered independently of policy: a target `baley.toml` never supplies settings. An unestablished target reads and writes no remembered denials.

| Field | Type | Meaning |
|---|---|---|
| `project_root` | path | The session project's canonical repository root |
| `checkout_root` | path or null | The canonical checkout root used for the commit. At `Cwd`, this comes from the cwd walk and is null when that walk fails or finds no checkout. For a redirected commit, it comes from the resolved target's walk and is null only when that walk finds no checkout; a failed or missing target walk makes the decision unrecordable. A root that is not UTF-8 is unrecordable in either case |
| `host` | text | `claude-code` |
| `refuse` | bool | Whether `git.on_protected` was `refuse` |
| `hard_fail` | bool | Whether `git.guard_hard_fail` was on |
| `protected_branches` | list | `git.protected_branches`, kept only when `refuse` or `hard_fail` is set, and empty otherwise |

### Views

Both views live in `user`, and each keeps the latest event for its key.

| View | Key | Content |
|---|---|---|
| `guard` | host, session (`""` for none), call id | The confirmed answer: input digest, project directory, working directory, outcome, reason and the event's sequence. Read before git for a redelivery, and again inside the audit transaction |
| `guard_policy` | session project root, target checkout root (`""` for none), host | The remembered denials only: `refuse`, `hard_fail`, and the protected list when either is set. Read only under torn settings, inside the audit transaction |

## 7. States

```mermaid
stateDiagram-v2
  [*] --> Received: hook call
  Received --> Silent: no project (a PowerShell call included), or no commit or push verb, or a declined or unreadable command, or a Monitor watch
  Received --> Replayed: in a project, an answer recorded under this call id for the same input, project directory and cwd
  Received --> Ask: PowerShell call in a project
  Received --> Deciding: project found, verb found
  Deciding --> Ask: push, or commit target unestablished, or protected commit under ask, or torn settings
  Deciding --> Deny: protected commit under refuse, or hard fail, or remembered denial
  Deciding --> PassOnFailure: git or branch unreadable
  Deciding --> Pass: unprotected commit, or protected commit under allow
  Ask --> Recorded: guard.answered in user
  Deny --> Recorded: guard.answered in user
  PassOnFailure --> Recorded: guard.answered in user
  Ask --> Unrecordable: store read or write fails, busy or needs rebuild, no envelope or call id, caller text cannot be recorded as given, call id answered for other input or directories, redirected target root unreadable or not walked, target or session project or cwd checkout root not UTF-8, settings without a session project, torn settings without branch or verb, successful transaction without a staged or replayed answer (call-id clash stderr line), or home folder unresolved
  Deny --> Unrecordable: the same causes
  PassOnFailure --> Unrecordable: the same causes
  Unrecordable --> Answered: an ask becomes a deny, a deny or a pass on failure stands, with one loud stderr line
  Recorded --> Answered: host form rendered
  Replayed --> Answered: the recorded answer rendered
  Silent --> [*]
  Pass --> [*]
  Answered --> [*]
```

*Figure 1. States of one command guard call. A commit is judged on the branch at its resolved target, with policy from the session project. An unestablished target asks without either read. A path call that the file or read guard denies is recorded and answered the same way. The lookup for a replay comes before any git or policy read, and the audit transaction looks again before it appends, so a call that another delivery answered first gets that answer instead of a second record.*

## 8. Workflows

```mermaid
sequenceDiagram
  participant A as Agent
  participant H as Host
  participant G as Baley guard
  participant L as Ledger
  participant O as Owner
  A->>H: Bash: git push origin main
  H->>G: hook (tool, command, cwd, session, call id), with CLAUDE_PROJECT_DIR set
  G->>G: walk up from CLAUDE_PROJECT_DIR to baley.toml
  alt no project
    G-->>H: nothing (pass)
    H->>A: runs
  else
    G->>G: scan: verb push
    G->>L: look up the call id in the guard view of user
    alt answered before for the same input, project directory and cwd
      L-->>G: the recorded answer
      G-->>H: that answer, with no git or policy read
    else answered before for other input
      L-->>G: the recorded answer, which does not match
      G-->>H: deny, reason: could not record, with a loud stderr line
      H->>A: blocked
    else not recorded yet
      G->>L: in user, read the guard view again, then append guard.answered ask
      alt the record fails, the store is busy or needs a rebuild, or the call id was answered for other input
        G-->>H: deny, reason: could not record, with a loud stderr line
        H->>A: blocked
      else recorded
        G-->>H: ask, reason
        H->>O: allow this push?
        O->>H: yes or no
      end
    end
  end
```

*Figure 2. A push through the guard. The lookup and the record are two short store opens, and the record is one `guard.record` command in `user` at policy version 0. A call id the lookup finds answered for other input takes no second open. A call with no call id never reaches the record, and its ask is denied the same way.*

```mermaid
sequenceDiagram
  participant A as Agent
  participant H as Host
  participant G as Baley guard
  participant L as Ledger
  A->>H: Bash: git commit -S -m "feat(T3): ..."
  H->>G: hook
  G->>G: project from CLAUDE_PROJECT_DIR, scan commit and its target
  G->>L: look up the call id in the guard view of user
  break answered before for the same input, project directory and cwd
    G-->>H: the recorded answer, with no git or policy read
  end
  break commit target unestablished
    G->>L: in the audit transaction, append guard.answered ask with no branch or settings
    G-->>H: ask, cannot tell which checkout the commit lands in
  end
  G->>G: branch at the resolved commit directory, then the session project's bounded settings with Claude Code's sections
  alt settings torn
    G->>L: in the audit transaction, the guard_policy view for this session project root, target checkout root and host
    alt remembered denial for this branch
      G->>L: guard.answered deny
      G-->>H: deny
    else
      G->>L: guard.answered ask
      G-->>H: ask, naming the torn file
    end
  else branch unreadable
    alt guard_hard_fail and branch provably protected
      G->>L: guard.answered deny
      G-->>H: deny
    else
      G->>L: guard.answered pass-on-failure
      G-->>H: nothing, loud stderr line
    end
  else protected branch
    alt on_protected refuse
      G->>L: guard.answered deny
      G-->>H: deny
    else ask
      G->>L: guard.answered ask
      G-->>H: ask
    else allow
      G-->>H: nothing
    end
  else other branch
    G-->>H: nothing
  end
```

*Figure 3. A commit through the guard. The branch and `.git/HEAD` fallback use the directory reached by the commit's `-C` chain, or `cwd` when none is given. Policy stays with the session project. The existing nullable `target` field records a redirected directory, and an unestablished target records an ask with no branch or settings. Every record here is made in the audit transaction in `user`, after the call's `guard` document is read again. Under a complete policy the same transaction appends `guard.policy_recorded` when the policy's denials differ from those remembered for this session project root, target checkout root and host. A plain pass records nothing and remembers nothing. When a record cannot be made, an ask becomes a deny and a deny or a pass on failure stands, as in Figure 2.*

## 9. Settings

| Setting | Type | Default | Scope | Owner | Effect |
|---|---|---|---|---|---|
| `git.protected_branches` | list of branch names | `["main", "master"]` | project | 0010 | Branches a commit is asked about or refused on (GRD-R5) |
| `git.on_protected` | `ask`, `refuse`, `allow` | `ask` | project | 0010 | What a commit on a protected branch gets (GRD-R5) |
| `git.guard_hard_fail` | bool | `false` | project | 0010 | Deny instead of pass when inputs are unreadable on a provably protected branch (GRD-R6) |

`git.protected_branches` is a TOML array of non-empty branch names, and `[]` protects no branch. A bare string, an entry that is not a string and a blank entry each make the file unavailable. So does a `git.on_protected` other than `ask`, `refuse` or `allow`, and a `git.guard_hard_fail` that is not a boolean. An unavailable file is torn settings, which GRD-R5 and GRD-R7 answer. `baley config set` writes the two scalars but refuses the list, naming `baley.toml`, where the owner edits it ([0003](0003-configuration-and-routing.md) section 5).

## 10. Instructions served

Not applicable. The guard gives no instructions; an agent sees only the host's own message for a held or blocked call, carrying the guard's reason.

## 11. Build status

`baley guard` runs the library hook (`crates/baley/src/main.rs:127`, `crates/baley/src/guard_hook/mod.rs:36-150`). It reads one call from standard input, binds the project from `CLAUDE_PROJECT_DIR`, judges the call with `baley-core`'s guard module (`crates/baley-core/src/guard/`) and the library's `hook_input` and `protected_paths` modules, records what it must in the per-user project `user`, and answers in Claude Code's hook form. The inherited guard is gone, and with it its `.planning` walk, its JSON settings, its `.planning`, `config.v4.json` and rendered-file protections, and its audit in the old store. What stays open is owned elsewhere: the placement projection that adds the placed stubs, settings and registration files and the executable to the protected list is built (`crates/baley/src/host_artifacts/placement.rs:271-296`) and T15 passes it to `judge`, the lease is Build 5's, and the doctor's runtime part is built (`crates/baley/src/host_doctor/`) while T17 plans its delivery part. The 2026-10-08 runs against Claude Code 2.1.294 showed how Claude Code handles the answers they reached (GRD-R12, first sheet `claude-live-2026-10-08-run1.md#ctl.commit-main-bash`) and saw no live redelivery (GRD-R10, `claude-live-2026-10-08-run1.md#ctl.redelivery`).

| Requirement | Status | Where |
|---|---|---|
| GRD-R1 | Partly built | One Claude Code `PreToolUse` entry runs `baley guard` with a 10-second timeout for `Bash\|Monitor\|PowerShell\|Read\|Grep\|Glob\|Write\|Edit\|NotebookEdit` (`hooks/hooks.json:3-14`), and the library classifies all nine tools (`crates/baley/src/hook_input/mod.rs:101-195`). The hook renderer gives the same item, with the nine-tool matcher as a constant, the guard's timeout and the absolute executable single-quoted before `guard` (`crates/baley/src/host_artifacts/hook.rs:13-43`), and a test holds the file's matcher and timeout to the renderer's (`crates/baley/src/host_artifacts/hook.rs:104-118`). The file stays hand-written, with a bare `baley guard` command, until T15 writes the hook from the renderer. In the 2026-10-08 runs the rendered nine-tool hook fired for `Bash`, `Monitor` and `Read` (first sheet `claude-live-2026-10-08-run1.md#ctl.hook.bash`, `#ctl.hook.monitor`, `#ctl.hook.read`). In the first run `Write`, `Edit` and `NotebookEdit` never reached the hook, because Claude Code's deny rules and its read-first check refused every call the procedure sent into the two folders (`claude-live-2026-10-08-run1.md#ctl.hook.write`, `#ctl.hook.edit`, `#ctl.hook.notebookedit`). In the third run those three fired the hook once each on unprotected files in the project (`claude-live-2026-10-08-run3.md#ctl.hook.write`, `#ctl.hook.edit`, `#ctl.hook.notebookedit`). `Grep` and `Glob` are not in Claude Code 2.1.294's default tool list. In the third run a session launched with `--allowedTools Grep,Glob` offered them, and the hook fired four times for each (`claude-live-2026-10-08-run3.md#ctl.hook.grep`, `#ctl.hook.glob`). `PowerShell` is unverified because `pwsh` was absent (`claude-live-2026-10-08-run1.md#ctl.hook.powershell`, `claude-live-2026-10-08-run3.md#ctl.hook.powershell`). The `/hooks` panel does not list a hook given through `--settings`, though the hook fires (`claude-live-2026-10-08-run1.md#ses.a.panels`) |
| GRD-R2 | Built | The hook judges `CLAUDE_PROJECT_DIR` with the server's rules (`crates/baley/src/mcp/context.rs:154-167`) and binds a project only when the walk from it finds a managed checkout. The walk treats only not-found as absence for both names, so an unusable nearest `baley.toml` entry still binds the project and a `.git` entry makes the folder the repository root and ends the walk (`crates/baley/src/discovery.rs:44-97`). The walk from the working directory gives the `baley.toml` protected for path tools, never the project. The context walk binds the project and the cwd checkout (`crates/baley/src/guard_hook/context.rs:44-132`). The judge resolves a commit's target directory, the entry reads its branch and walks that directory, and the recorder chooses the checkout root for remembered denials (`crates/baley/src/guard_hook/decide.rs:115-134`, `crates/baley/src/guard_hook/mod.rs:106-110`, `crates/baley/src/guard_hook/record.rs:102-134`). With no project bound, a command or `PowerShell` call passes with nothing recorded, and path calls are judged at the working directory either way (`crates/baley/src/guard_hook/decide.rs:80-204`, `crates/baley-core/src/guard/answer.rs:61-70, 138-144`) |
| GRD-R3 | Built | The core scanner returns `GitCommand::Commit(CommitTarget)` or `Push`, with ordered `-C` operands and explicit `Cwd`, `Directory` and `Unestablished` targets (`crates/baley-core/src/guard/scan.rs:13-220`), the PowerShell ask is a core decision (`crates/baley-core/src/guard/answer.rs:133-144`), and the library classifies `Bash`, `Monitor` command and watch forms, and `PowerShell` (`crates/baley/src/hook_input/mod.rs:101-195`). The hook scans a `Bash` or `Monitor` command and passes a declined command or a watch with nothing recorded (`crates/baley/src/guard_hook/decide.rs:81-114`). Claude Code sent `command`, `description` and `timeout_ms` for the `Monitor` command form and `ws`, `description` and `timeout_ms` with no `command` for its watch form, and the names for `PowerShell` are unverified (section 12, `claude-live-2026-10-08-run1.md#fld.monitor`, `#fld.monitor-watch`, `#fld.powershell`). A commit with shell structure the scanner declines passed with nothing recorded and landed on `main` (`claude-live-2026-10-08-run1.md#ctl.declined-syntax`) |
| GRD-R4 | Built | The core answer (`crates/baley-core/src/guard/answer.rs:61-73`). The hook asks on every push in a project before it reads a branch or a settings file (`crates/baley/src/guard_hook/decide.rs:115-118`) |
| GRD-R5 | Built | The core answer, with the settings read from a complete policy (`crates/baley-core/src/guard/answer.rs:82-102`, `crates/baley-core/src/guard/settings.rs:12-54`) and the three settings in the schema (`crates/baley-core/src/policy/schema.rs:296-316`). The hook reads the session project's policy with Claude Code's host sections (`crates/baley/src/guard_hook/policy.rs:31-107`) and judges a commit on the branch git read at its target directory. An unestablished target selects a recorded ask with a fixed reason before any branch or policy step (`crates/baley/src/guard_hook/decide.rs:115-149`, `crates/baley-core/src/guard/reason.rs:23-26`) |
| GRD-R6 | Built | The core answer (`crates/baley-core/src/guard/answer.rs:90-100`). The hook asks git for the branch at the resolved commit directory on the guard's budget, reads `.git/HEAD` without following links beside it, and judges the two (`crates/baley/src/guard_hook/branch.rs:23-141`, `crates/baley/src/guard_hook/mod.rs:106-110`). A pass on failure is recorded (`crates/baley/src/guard_hook/decide.rs:242-254`) and printed as one stderr line (`crates/baley/src/guard_hook/render.rs:56-62`). Claude Code's hooks reference sends a hook's stderr on exit 0 to its debug log only, so where the owner sees that line stays unobserved: no loud line was produced in the 2026-10-08 runs (`claude-live-2026-10-08-run1.md#ctl.stderr-line`). The `.git/HEAD` fallback was tried by taking git off `PATH`. The guard then could not read HEAD's copy of `baley.toml`, so it asked on torn settings (GRD-R7) and the pass-on-failure path was not reached, which leaves the fallback name's rule unverified live (`claude-live-2026-10-08-run1.md#ctl.fallback-head`). The doctor carries none of the hook's live observations: they stay in the dated host-matrix record cited here |
| GRD-R7 | Built | The shared settings reader refuses a dangling link as a link to a missing file with `config-unavailable`, including a global `config.toml` link whose target is missing (`crates/baley/src/settings.rs:111-135`). The hook treats that global file and an unreadable or non-regular nearest `baley.toml` as torn settings and asks, subject to the remembered denials (`crates/baley/src/guard_hook/policy.rs:61-107`). The core answer for torn settings and the two remembered denials (`crates/baley-core/src/guard/answer.rs:104-131`), and the denials kept, their change judge and the `guard_policy` view (`crates/baley-core/src/guard/remembered.rs:28-226`). Under torn settings the hook reads the denials remembered for the session project root, the target checkout root and Claude Code inside the audit transaction and judges the commit again with them, and a complete policy appends `guard.policy_recorded` beside its answer only when its denials changed (`crates/baley/src/guard_hook/record.rs:102-151, 286-354`) |
| GRD-R8 | Built | The two events, their payloads and the input digest (`crates/baley-core/src/guard/event.rs:19-42, 63-91, 104-181`). The hook selects every ask, deny and pass on failure of a call with an envelope, with its input digest, a path tool's target or a redirected commit's resolved directory in the existing nullable `target`, and a commit's facts (`crates/baley/src/guard_hook/decide.rs:62-78, 115-149, 242-254`), and records it as one `guard.record` command in `user` at policy version 0, by `baley`, with the hook caller, creating `user` when missing (`crates/baley/src/guard_hook/record.rs:165-284, 356-371`). A torn HEAD copy's cause is kept with `[redacted]` in place of git's excerpt, which the bounded reader gives apart (`crates/baley/src/committed.rs:84-137`). Both events stay at version 1, since their field shapes and readers are unchanged. Both views stay registered at view set 7 (`crates/baley/src/ledger/open.rs:15-44`) |
| GRD-R9 | Built | The core mapping from an answer and the audit precondition (`crates/baley-core/src/guard/recording.rs:27-34`). The hook builds the call's identity from its envelope and `CLAUDE_PROJECT_DIR`, or the line saying why it cannot, and invents none (`crates/baley/src/guard_hook/unrecordable.rs:12-41`). Every store failure is unrecordable with its cause named, and needs-rebuild names `baley rebuild user` (`crates/baley/src/guard_hook/unrecordable.rs:43-58`). A call id answered for other input is unrecordable (`crates/baley/src/guard_hook/record.rs:215-236`, `crates/baley/src/guard_hook/unrecordable.rs:60-64`), and a new answer counts as recorded only once its transaction commits (`crates/baley/src/guard_hook/record.rs:275-283`). When that transaction returns successfully without staging or replaying an answer, the entry prints the call-id clash line (`crates/baley/src/guard_hook/record.rs:281-282`, `crates/baley/src/guard_hook/mod.rs:138-141`). Missing recording facts or a policy key that cannot be represented as UTF-8 are unrecordable, as is a redirected target that could not be walked (`crates/baley/src/guard_hook/record.rs:102-151`). Without Baley's home folder nothing is recorded, and the precondition decides the final answer (`crates/baley/src/guard_hook/mod.rs:73-81, 123-150`). No unrecordable answer was produced in the 2026-10-08 runs, so where the owner sees the loud line stays unobserved (`claude-live-2026-10-08-run1.md#ctl.stderr-line`). The doctor reports the `user` project's views that were behind this binary's when it started and names `baley rebuild user` (`crates/baley/src/host_doctor/guard_records.rs:38-78`, `crates/baley/src/host_doctor/report.rs:220-233`) |
| GRD-R10 | Built | The `guard` view, its projector and the redelivery judge (`crates/baley-core/src/guard/redelivery.rs:13-205`). A command with a commit or push verb, or a `PowerShell` call, in a project is looked up on a short store open of its own before any git or policy read (`crates/baley/src/guard_hook/decide.rs:87-93, 108-114`, `crates/baley/src/guard_hook/mod.rs:88-105`, `crates/baley/src/guard_hook/record.rs:378-398`), and the audit transaction reads the view again before it appends (`crates/baley/src/guard_hook/record.rs:215-236`). No `tool_use_id` arrived twice in either 2026-10-08 run, so a live redelivery was not observed and stays unverified (`claude-live-2026-10-08-run1.md#ctl.redelivery`, and the timing read in `claude-live-2026-10-08-run2.md#post-run-reads`) |
| GRD-R11 | Partly built | The library write decision and its lease input (`crates/baley/src/protected_paths/write.rs:18-101`), with path resolution and containment (`crates/baley/src/protected_paths/resolve.rs:151-238`, `crates/baley/src/protected_paths/contain.rs:14-129`). A protected file that does not exist yet is matched through its deepest existing ancestor, so a case-variant spelling cannot create it (`crates/baley/src/protected_paths/write.rs:111-133`, `crates/baley/src/protected_paths/contain.rs:44-75`). The hook judges `Write`, `Edit` and `NotebookEdit` at the working directory with or without a project, and denies them when Baley's folders cannot be resolved (`crates/baley/src/guard_hook/decide.rs:154-204`). It protects the home and config folders, the session project's `baley.toml` and the one that binds the working directory's checkout (`crates/baley/src/guard_hook/context.rs:77-132`); production `judge` lists only those two `baley.toml` files (`crates/baley/src/guard_hook/context.rs:106-114, 126-130`). The placement projection is built and holds the placed stubs, the placed settings and registration files and the installed executable (`crates/baley/src/host_artifacts/placement.rs:271-296`), and T15 passes it in at `judge`. T14's staged-version folder is planned, not built. Build 5 owns the lease |
| GRD-R12 | Built | The renderer (`crates/baley/src/guard_hook/render.rs:6-99`) and the hook's write of its answer, with the blocking exit when stdout fails (`crates/baley/src/guard_hook/mod.rs:174-197`). In the 2026-10-08 runs Claude Code obeyed `deny` for a commit on `main` through `Bash` and `Monitor`: the command did not run and `git log` was unchanged (first sheet `claude-live-2026-10-08-run1.md#ctl.commit-main-bash`, `#ctl.commit-main-monitor`, and `claude-live-2026-10-08-run2.md#ses.a.smoke-commit`). It obeyed `ask` for a push through both tools: a yes ran the push and a no rejected the call and left the remote's refs as they were (`claude-live-2026-10-08-run1.md#ctl.push-bash-yes`, `#ctl.push-bash-no`, `#ctl.push-monitor-yes`, `#ctl.push-monitor-no`). A pass with no output let the call run (`claude-live-2026-10-08-run1.md#ctl.declined-syntax`). In the third run the guard denied `Grep` and `Glob` calls over a parent of Baley's folders, and Claude Code refused each call and showed the hook's message as the result (`claude-live-2026-10-08-run3.md#ctl.grep-parent-guard`, `#ctl.glob-parent-guard`). The answers for `PowerShell`, and for `Write`, `Edit` and `NotebookEdit` over a protected file, were not reached and are unverified (`claude-live-2026-10-08-run1.md#ctl.powershell-ask`, `#ctl.write-baleytoml-denied`). The doctor carries no live evidence: how Claude Code honours each answer is the dated record cited here |
| GRD-R13 | Partly built | The library read decision (`crates/baley/src/protected_paths/read.rs:22-105`), which the hook calls for `Read`, `Grep` and `Glob` at the working directory with or without a project (`crates/baley/src/guard_hook/decide.rs:193-195`), and which the matcher puts in front of those tools (`hooks/hooks.json:5`). The sandbox and `Read` and `Edit` deny-rule content is rendered (`crates/baley/src/host_artifacts/security.rs:142-199`) and composed into an existing settings document (`crates/baley/src/host_artifacts/compose.rs:98-126, 189-270`). Where the content is composed, Baley's secure values replace a disabled sandbox, `failIfUnavailable` false and `allowUnsandboxedCommands` true, each replacement reported. Composition reports, and keeps, `sandbox.filesystem.disabled` or `disableAllHooks` true, those three switches left insecure in a document the content is not composed into, every `sandbox.excludedCommands` entry, a folder re-opened by an absolute `allowRead` or `allowWrite` entry, and a `~/` or relative entry it cannot judge. The coverage judge reports from a settings document, per tool and per read or write, which mechanism covers each folder, with Grep and Glob best-effort (`crates/baley/src/host_artifacts/coverage.rs:281-356`). Any excluded command leaves the shell tools and the write-only files a gap (`crates/baley/src/host_artifacts/coverage.rs:375-398`), and a `~/` or relative allow entry is a gap reported as not judged (`crates/baley/src/host_artifacts/coverage.rs:429-442`). Both read only the settings they name, in the one document they are given, so a setting such as `sandbox.enabledPlatforms` leaving out this platform, or one in another settings file, is not seen. T15 applies the content. The doctor runs the same judge over the settings and hook documents it is given and words each verdict as configuration of the named document, not as proof (`crates/baley/src/host_doctor/protection.rs:48-92`, `crates/baley/src/host_doctor/report.rs:265-322`), and it checks that the guard runs for all nine tools of the hook's matcher (`crates/baley/src/host_doctor/protection.rs:97-161`). Until T15 supplies a placement it reports every artifact as not installed, and T17 plans the delivery checks over the installed result. The sandbox probe is a spike (`spikes/host-matrix`). The 2026-10-08 runs extended it to separate home and config folders and the built-in file tools: Bash, its children and a Monitor child could neither read nor write either folder, and `Read`, `Write` and `Edit` naming a file there were refused by the deny rules before the hook ran, so the guard's own decision for those calls was not reached (first sheet `claude-live-2026-10-08-run1.md#bar.home.bash-read`, `#bar.config.bash-write`, `#bar.home.monitor-child-read`, `#bar.config.read-tool`, `#bar.config.write-tool`). `Grep` and `Glob` are not in that Claude Code version's default tool list, so the first two runs could not test them. In the third run, in a session launched with `--allowedTools Grep,Glob`, the guard denied both over each folder itself and over its parent, for the home and for the config folder, and the session showed the refusal with nothing of the folder's content (third sheet `claude-live-2026-10-08-run3.md#bar.home.grep-folder`, `#bar.home.grep-parent`, `#bar.home.glob-folder`, `#bar.home.glob-parent`, `#bar.config.grep-folder`, `#bar.config.grep-parent`, `#bar.config.glob-folder`, `#bar.config.glob-parent`, `#ctl.grep-parent-guard`, `#ctl.glob-parent-guard`). Over the folder itself the guard answered before the best-effort `Read` rule, so that rule's own reach over `Grep` and `Glob` was not measured and stays unverified |
| GRD-R14 | Partly built | The hook starts the budget before it reads standard input, and reads one byte past the bound (`crates/baley/src/guard_hook/mod.rs:50-61`). The classifier applies the bound to every tool and denies unreadable input for the six path tools (`crates/baley/src/hook_input/mod.rs:19-21, 133-149, 225-233`). The budget's limits and grants are pure decisions over supplied times (`crates/baley/src/guard_budget.rs:20-115`). Git for the branch launches only on a grant from it and is charged the time it took, and `.git/HEAD` is opened without following links or blocking, read only when it is a regular file, and capped at 4,096 bytes (`crates/baley/src/guard_hook/branch.rs:23-141`). The launch validator accepts a guard caller's git only at the timeout its grant gave it, above zero and at most 5 seconds, in its own process group, and every other caller only at its exact registered deadline (`crates/baley/src/process.rs:128-138, 233-274`, `crates/baley/src/git_process.rs:49-99`). The bounded reader of HEAD's copy of `baley.toml` launches under its own guard caller on the same git allowance, keeps at most 1 MiB of each stream and keeps at most 256 bytes of git's first stderr line for its error (`crates/baley/src/committed.rs:75-137, 200-208`), and the global and working-tree files are read capped at 1 MiB (`crates/baley/src/settings.rs:42-57, 137-148`). Each of the hook's two store opens takes the storage time the budget has left and is charged the time it took (`crates/baley/src/guard_hook/mod.rs:152-167`). The guard's storage options (`crates/baley/src/ledger/open.rs:56-69`) open a store that takes its writer queue and connections by nonblocking tries until its storage time is spent and sets each statement's busy timeout to the time left (`crates/baley-store-sqlite/src/queue.rs:73-90, 175-242`, `crates/baley-store-sqlite/src/store.rs:31-65, 423-454, 687-700`). Such a store takes its maintenance lock the same bounded way (`crates/baley-store-sqlite/src/store.rs:416-421`), but the guard path never asks for it: it answers views behind this binary's as needs-rebuild instead of rebuilding them (`crates/baley-store-sqlite/src/rebuild.rs:83-103`). The hook starts no async runtime. Each guard process's write to `user` checks that every event type in `user`'s chain is readable, because a store keeps the head it last checked only in memory (`crates/baley-store-sqlite/src/transact.rs:370-411`). The cost of that check grows with `user`'s chain and nothing bounds it yet, so the 10-second answer is not guaranteed. The 2026-10-08 runs measured the hook's times while other sessions exited: guard calls took 46 to 47 ms in the first run and 45 to 47 ms in the second, with a highest of 48 and 49 ms, none near the timeout (`claude-live-2026-10-08-run1.md#ctl.contention-exit`, `#ctl.latency`, `claude-live-2026-10-08-run2.md#ctl.contention-exit`, `claude-live-2026-10-08-run2.md#post-run-reads`). The `user` chain held only 48 events in the first run (`claude-live-2026-10-08-run1.md#post.verify-user`), so this is the cost on a short chain, and Build 9 bounds the check on a long one |

## 12. Open questions

Some of the guard's tool-input facts below are still not measured on Claude Code. The stand-in probe's hook input, read in the 2026-10-08 runs, shows the `tool_input` fields of `Bash`, `Monitor` in both forms, `Read`, `Write`, `Edit` and `NotebookEdit`, and every name matches section 5 (first sheet `claude-live-2026-10-08-run1.md#fld.bash`, `#fld.monitor`, `#fld.monitor-watch`, `#fld.read`, `#fld.write`, `#fld.edit`, `#fld.notebookedit`). `Grep` and `Glob` are not in Claude Code 2.1.294's default tool list, so the first run did not see them. In the third run the stand-in probe's session launched with `--allowedTools Grep,Glob` showed `Grep`'s `pattern` and `path` and `Glob`'s `pattern` and `path`, and the names the guard reads match section 5 (`claude-live-2026-10-08-run3.md#fld.grep`, `#fld.glob`). `PowerShell` was not seen, because `pwsh` was absent (`claude-live-2026-10-08-run1.md#fld.powershell`), and neither was `Grep`'s `glob`, because the call gave none. Until they are seen, the guard applies the rule stated with each item.

- **Field names.** The live hook input shows `Monitor`'s watch form sending `ws`, `description` and `timeout_ms` with no `command`, its command form sending `command`, `description` and `timeout_ms`, and `NotebookEdit` sending `cell_id`, `new_source` and `notebook_path`. All three match section 5, so none needed a change (first sheet `claude-live-2026-10-08-run1.md#fld.monitor-watch`, `#fld.monitor`, `#fld.notebookedit`). The third run showed `Grep` sending `pattern` and `path` and `Glob` sending `pattern` and `path`, which match section 5 (third sheet `claude-live-2026-10-08-run3.md#fld.grep`, `#fld.glob`). No tool call seen so far shows the `tool_input` fields of `PowerShell` or `Grep`'s `glob`. For those the guard reads the names section 5 lists. A path tool that lacks a field it requires (`file_path`, `notebook_path` or `Glob`'s `pattern`) is denied, so a wrong name for one of those fails closed. `Grep`'s `path` and `glob` and `Glob`'s `path` are optional, so a wrong name for one of them is read as absent: the call is judged against the hook's working directory, or without its `glob`, and can pass. The names of `Grep`'s `path` and `Glob`'s `path` are now seen, so that risk remains only for `Grep`'s `glob`. A `PowerShell` call is judged by its tool name and none of its input is read, so its input digest holds that name alone. Claude Code's hooks reference, read on 2026-10-06, says `PowerShell` sends `command` with the same fields as `Bash`, which the guard leaves unread by design, and the live runs have not seen it.
- **Pattern reach.** Whether a `Glob` `pattern` or a `Grep` `glob` can reach outside `path`, and whether either follows symbolic links inside a searched folder, is not known. Meanwhile the guard checks the folders a pattern names before its first wildcard whatever `path` is, refuses a `..` component after a wildcard, and does not look inside the searched folder. Links inside it are left to the sandbox and the `Read` deny rules (GRD-R13).
- **Absent path.** Whether an absent `Grep` or `Glob` `path` means the hook's working directory is not known. Meanwhile the guard takes it as the working directory.
- **On-disk case.** Whether `canonicalize` returns the on-disk case of a path on a case-insensitive macOS volume is not known. The identity check in GRD-R11 and GRD-R13 compares existing ancestors by (device, inode), so it covers either answer.
