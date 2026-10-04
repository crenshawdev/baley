# 0010: Guard

| | |
|---|---|
| Status | Accepted |
| Design issue | none; build issue [#24](https://github.com/crenshawdev/baley/issues/24) |
| Requirement prefix | GRD |
| Applies | [0002: System design](0002-system-design.md) |
| Related | ADRs: [0008](../adr/0008-host-sandbox-isolation.md), [0009](../adr/0009-served-instructions.md), [0027](../adr/0027-vendor-folders-and-plain-keys.md), [0033](../adr/0033-host-security-bar.md) · C4 view: components ([0002](0002-system-design.md) Figure 4) |

The current design of this area, and nothing else. Edit it in place when the design changes; git holds the history. It describes the design only, never the work still to do.

## 1. Purpose and scope

This area decides what Baley does at the host's edge, before a tool call an agent makes runs:

- the one hook Baley installs, what it sees on Claude Code, and how it finds the project;
- the command guard, for `Bash`, `Monitor` and `PowerShell` commands: which git commands it acts on, what it asks, what it refuses, and what it does when it cannot decide;
- the file guard, for `Write`, `Edit` and `NotebookEdit`: which paths an agent may not write;
- how each answer is recorded, remembered and replayed;
- the answer adapter that renders an answer in the host's hook form, and the protection that keeps agents from reading or writing Baley's home and its config folder.

It does not decide the lease itself or what an out-of-lease commit does at task close ([0006](0006-execution.md)); which branch work happens on or how landing pushes ([0011](0011-milestones-landing-undo-pause.md)); risk detection ([0009](0009-risk.md)); or how stubs are rendered and installed ([0012](0012-host-interface.md)).

Hand-offs: 0003 gives the guard project discovery and the settings it reads; 0006 gives it the active lease; 0012 installs the hook and the sandbox configuration; the ledger (0001) stores every guard outcome.

In the component view of [0002](0002-system-design.md) (Figure 4) the guard is the third way into the host interface, beside the MCP server and the command line.

## 2. Terms

| Term | Meaning |
|---|---|
| Hook | Claude Code's pre-tool-use call into Baley, for `Bash`, `Monitor`, `PowerShell`, `Read`, `Grep`, `Glob`, `Write`, `Edit` and `NotebookEdit`. The host passes the tool, its input, the working directory, the session and a call id. |
| Guard | Baley's answer to a hook call: `pass`, `ask`, `deny`, or `pass on failure`. |
| Verb | The git subcommand a `Bash` or `Monitor` command carries: `commit` or `push`. A `PowerShell` command is not scanned. |
| Protected branch | A branch named in `git.protected_branches`. |
| Torn settings | A settings file that cannot be read or parsed at the moment of the call, or that holds a value outside its type or grammar (CFG-R9), such as a bare-string branch list or an unknown `git.on_protected`. |
| Guard failure | A call the guard could not decide because git or the branch could not be read. |
| Hard fail | The opt-in rule (`git.guard_hard_fail`) that turns a guard failure on a provably protected branch into a deny. |
| Remembered policy | The last complete, successfully read policy for the project, kept so that a denial can still be given when the settings are torn. It never supplies an allow. |
| Redelivery | The host calling the hook again with the same call id after a timeout. |
| Answer adapter | The translation of a guard answer into the host's hook form. Claude Code takes `allow`, `deny` and `ask`. |
| Sandbox | The host's own restriction on what an agent process may read and write ([ADR 0008](../adr/0008-host-sandbox-isolation.md)). |

## 3. Requirements

| Id | Rule | Why | Depends on | Status |
|---|---|---|---|---|
| GRD-R1 | Baley installs one hook: Claude Code's pre-tool-use call for `Bash`, `Monitor`, `PowerShell`, `Read`, `Grep`, `Glob`, `Write`, `Edit` and `NotebookEdit`, with a bounded timeout. There is no other hook. | One edge for every tool that runs a shell command or reads or writes a file. | SYS-P11, SYS-P12 | Active |
| GRD-R2 | The guard takes the project from the hook's `CLAUDE_PROJECT_DIR`, walking up from it to the nearest `baley.toml` and stopping at the git repository root (CFG-R4). The hook's working directory resolves the paths a tool call names and names the checkout whose branch is read, so a `/cd` never changes the project. Outside a project the guard is silent for `Bash`, `Monitor` and `PowerShell` calls. The path rules need no project and apply either way: the `Write`, `Edit` and `NotebookEdit` refusal (GRD-R11) and the refusal of a `Read`, `Grep` or `Glob` call that reaches Baley's home or its config folder (GRD-R13). | The guard acts only where Baley is responsible. | CFG-R4 | Active |
| GRD-R3 | The command guard reads the command of a `Bash` call and of a `Monitor` command watch with one scan, and acts on a command only when it carries a git `commit` or `push` verb. It splits the command on `;`, `|`, `&`, `&&`, `||` and newlines, takes segments whose first word is `git` or ends in `/git`, skips git's global flags and their operands, and declines to judge a command containing substitutions, backticks, redirects, subshells, braces, a leading comment, a NUL, an unclosed quote or a trailing backslash. A command that carries both verbs is judged as a push. A declined command passes with nothing recorded, and so does a `Monitor` watch with no command, which is not scanned. In a bound project a `PowerShell` call asks, whether or not it mentions git, and the reason names the grammar the scan cannot judge. A `PowerShell` call is never scanned. No other git verb is covered; this is a stated limit of the design. | Commit and push are where work reaches the record and the forge; everything else was tried and did not pay. Baley reads POSIX shell, so it asks about PowerShell instead of guessing. | | Active |
| GRD-R4 | A `push` always asks, on any branch, with a fixed reason. | Publishing is the owner's step. | SYS-P5 | Active |
| GRD-R5 | A `commit` on a protected branch follows `git.on_protected`: `ask`, `refuse` (deny) or `allow`, acting on the branch git read. An unknown value is outside the setting's grammar, so the file is torn (CFG-R9) and the commit asks, naming the file, unless a remembered denial applies (GRD-R7). On any other branch a commit passes. | The owner sets the branch discipline once. | CFG-R5, CFG-R9 | Active |
| GRD-R6 | When git or the current branch cannot be read, the guard passes, prints a loud line on stderr, and records a guard failure. When `git.guard_hard_fail` is set and the branch is provably protected from what could be read, it denies instead. | A guard that cannot decide must not silently become a wall or a hole. | | Active |
| GRD-R7 | When a settings file is torn, the guard asks, naming the file. Two denials from the remembered policy still stand: a remembered `refuse` on a branch git read that is in the remembered protected list, and a remembered hard fail on a provably protected branch when git cannot read the branch. A remembered `allow` or `ask` never relaxes the ask. The remembered policy keeps only denials, never an allow. | Torn settings must not open the door. | CFG-R9 | Active |
| GRD-R8 | Every `ask`, `deny` and guard failure is recorded before it is answered: the command digest (never the command), the working directory, the project, the verb, the branch, the policy in force, the outcome and the reason. A plain pass records nothing. | The record shows what the guard held and why. | SYS-P6 | Active |
| GRD-R9 | When the decision cannot be recorded, an `ask` becomes `deny` with the reason that the guard could not record its decision, on stderr and in the answer. A decision is unrecordable when the write fails, when the views need a rebuild, or when the call carries no call id to replay it by. A `deny` stays a deny, a pass on failure stays a loud pass and a plain pass stays a pass. A decision that was recorded is not changed, so a recorded ask about torn settings stays an ask. | An unrecorded ask is the gap the record exists to close. | GRD-R8 | Active |
| GRD-R10 | A redelivered call with the same call id gets its confirmed answer again, from the record, even after the policy changed. | The host may deliver a call twice; the answer must not differ. | EVD-R26 | Active |
| GRD-R11 | The file guard denies a `Write`, `Edit` or `NotebookEdit` call whose target is inside or is one of these protected paths: Baley's home folder; Baley's config folder (the global settings file `config.toml` and the keys file `keys.env`, [0003](0003-configuration-and-routing.md) CFG-R2, CFG-R24); the `baley.toml` of the session's project and the `baley.toml` of the checkout the hook's working directory is in, by path or by file identity, but no other file of that name; and the stub paths Baley supplies. It judges them from any working directory, with or without a project. Path resolution canonicalizes the existing prefix, reads both the spelling given and the spelling with each backslash taken as a slash, and refuses an empty value, control bytes, a leading `//` or drive-letter prefix, and a non-directory parent. It judges containment by component and by the (device, inode) identity of existing ancestors, so a sibling such as `baley-old` is not inside `baley` and a differently cased spelling on a case-insensitive volume is still caught. A path it cannot resolve is denied. During an active dispatch, a path outside the dispatch's lease is denied (EXE-R8). Build 5 owns the lease: the decision takes it as an input whose only value today is no active dispatch, and Build 5 defines what a lease names and the deny for a write outside it. | The owner sets policy and holds the keys, Baley renders stubs, and the lease means something as the write happens. | CFG-R11, CFG-R24, ADR 0009, EXE-R8 | Active |
| GRD-R12 | The answer adapter renders each answer in the host's hook form. Claude Code takes `allow`, `deny` and `ask`. | One guard decision, rendered in the host's form. | SYS-P8 | Active |
| GRD-R13 | Agents can neither read nor write Baley's home or its config folder. Three mechanisms carry it, configured at install: Claude Code's sandbox, for shell commands and their children (`Bash`, `Monitor`, `PowerShell`); its `Read` and `Edit` deny rules, for the built-in file tools; and the guard's refusal of a `Read`, `Grep` or `Glob` call that reaches either folder, because Claude Code applies `Read` rules to `Grep` and `Glob` only on a best-effort basis. The guard takes the call's target from `Read`'s `file_path` or from `Grep`'s or `Glob`'s `path`, which is the hook's working directory when absent. It also takes the folders a `Grep` `glob` or a `Glob` `pattern` names before its first wildcard, joined to that target. It refuses when a target lies inside either folder or holds one, compared by component and by file identity, so a sibling such as `baley-old` is not matched. It refuses a pattern with a `..` component after a wildcard, and a path it cannot resolve. An unavailable sandbox is reported, never passed over. A `keys.env` that is a symbolic link is covered only where its target lies inside those folders. `baley doctor` checks the configuration and reports what an agent can reach, reads included. | The record and the keys are protected by the host and the guard together, and tampering is detected by the chain and its anchors. | ADR 0008, ADR 0033 | Active |
| GRD-R14 | The guard reads the hook's input up to a fixed bound, answers within the hook's timeout, and never launches a program except git for the branch; when git is unavailable it reads the branch from `.git/HEAD` directly, bounded and without following symbolic links. Input it cannot read, or that is over the bound, is denied for `Read`, `Grep`, `Glob`, `Write`, `Edit` and `NotebookEdit`, and passes for the command tools. | The guard must answer fast and must not become a way to run things. | | Active |

## 4. Roles and actors

| Actor | Receives | Returns | Model and effort from |
|---|---|---|---|
| Host | The guard's answer in its own form | Runs, asks the owner, or blocks the tool call | Not applicable |
| Owner | An `ask` put by the host | Yes or no, in the host | Not applicable |
| Agent (any worker or the session) | A denied or held tool call with its reason | Nothing; it does not argue with the guard | Not applicable |
| Baley guard | The hook input | pass, ask, deny, pass on failure | Not applicable |
| Ledger | Guard records | | Not applicable |

No model is dispatched by this area.

## 5. Commands and operations

### baley guard (hook entry)

- **Inputs:** the hook's JSON on standard input: tool name, tool input (command, or file or notebook path and content), working directory, session id, call id.
- **Outputs:** nothing for pass and pass on failure; for ask and deny, Claude Code's permission form with the reason.
- **Refusals (as answers):**

  | Answer | When | Requirement |
  |---|---|---|
  | `ask` | push; protected commit under `ask`; torn settings | GRD-R4, GRD-R5, GRD-R7 |
  | `deny` | protected commit under `refuse`; hard fail; remembered denial under torn settings; a Write, Edit or NotebookEdit to a protected path or outside the lease; a `Read`, `Grep` or `Glob` call whose path lies inside or contains Baley's home or its config folder, with or without a project; an unrecordable ask | GRD-R5, GRD-R6, GRD-R7, GRD-R9, GRD-R11, GRD-R13 |
  | `pass on failure` | git or branch unreadable without hard fail | GRD-R6 |
  | `pass` | everything else, including a declined command and any other call outside a project | GRD-R2, GRD-R3 |

### baley doctor (the guard's part)

- **Inputs:** the host name.
- **Outputs:** whether the hook is installed and points at this binary; whether the sandbox keeps an agent from writing Baley's home and config folder, and from reading them; the answer forms the host honours.
- **Refusals:** none; findings are reported (GRD-R13).

## 6. Records

### guard.answered (event, `project` stream; `guard` stream for calls outside a project that still denied)

| Field | Type | Meaning |
|---|---|---|
| `call` | session id, call id | The hook call, for redelivery |
| `tool` | `bash`, `monitor`, `powershell`, `read`, `grep`, `glob`, `write`, `edit`, `notebookedit` | |
| `command_digest` or `path` | digest, path | Never the command text |
| `cwd` | path | |
| `verb` | `commit`, `push`, absent | |
| `branch` | name or unknown | |
| `policy` | table | `on_protected`, `protected_branches`, `guard_hard_fail` as read, and which file each came from, or `torn` |
| `outcome` | `ask`, `deny`, `pass-on-failure` | |
| `reason` | text | |
| `unavailable` | list | Inputs that could not be read |

### guard.policy_recorded (event, `project` stream)

The last complete policy read, kept for GRD-R7. The `guard_policy` view holds the latest per project.

### Views

| View | Key | Content |
|---|---|---|
| `guard` | project, call | The confirmed answer for redelivery |
| `guard_policy` | project | The remembered policy, denials only |

## 7. States

```mermaid
stateDiagram-v2
  [*] --> Received: hook call
  Received --> Silent: no project, or no commit or push verb, or declined command
  Received --> Deciding: project found, verb found
  Deciding --> Ask: push, or protected commit under ask, or torn settings
  Deciding --> Deny: protected commit under refuse, or hard fail, or remembered denial
  Deciding --> PassOnFailure: git or branch unreadable
  Deciding --> Pass: unprotected commit
  Ask --> Recorded: guard.answered
  Deny --> Recorded: guard.answered
  PassOnFailure --> Recorded: guard.answered
  Ask --> Deny: record could not be written
  Recorded --> Answered: host form rendered
  Silent --> [*]
  Pass --> [*]
  Answered --> [*]
```

*Figure 1. States of one command guard call.*

## 8. Workflows

```mermaid
sequenceDiagram
  participant A as Agent
  participant H as Host
  participant G as Baley guard
  participant L as Ledger
  participant O as Owner
  A->>H: Bash: git push origin main
  H->>G: hook (tool, command, cwd, session, call id)
  G->>G: walk up to baley.toml
  alt no project
    G-->>H: nothing (pass)
    H->>A: runs
  else
    G->>G: scan: verb push
    G->>L: guard.answered ask
    alt record fails
      G-->>H: deny, reason: could not record
      H->>A: blocked
    else
      G-->>H: ask, reason
      H->>O: allow this push?
      O->>H: yes or no
    end
  end
```

*Figure 2. A push through the guard.*

```mermaid
sequenceDiagram
  participant A as Agent
  participant H as Host
  participant G as Baley guard
  participant L as Ledger
  A->>H: Bash: git commit -S -m "feat(T3): ..."
  H->>G: hook
  G->>G: project, verb commit, read settings
  alt settings torn
    G->>L: guard_policy view
    alt remembered denial for this branch
      G-->>H: deny
    else
      G-->>H: ask, naming the torn file
    end
  else branch unreadable
    alt guard_hard_fail and branch provably protected
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

*Figure 3. A commit through the guard.*

## 9. Settings

| Setting | Type | Default | Scope | Owner | Effect |
|---|---|---|---|---|---|
| `git.protected_branches` | list of branch names | `["main", "master"]` | project | 0010 | Branches a commit is asked about or refused on (GRD-R5) |
| `git.on_protected` | `ask`, `refuse`, `allow` | `ask` | project | 0010 | What a commit on a protected branch gets (GRD-R5) |
| `git.guard_hard_fail` | bool | `false` | project | 0010 | Deny instead of pass when inputs are unreadable on a provably protected branch (GRD-R6) |

## 10. Instructions served

Not applicable. The guard gives no instructions; an agent sees only the host's own message for a held or blocked call, carrying the guard's reason.

## 11. Build status

The binary crate holds the inherited engine; its guard is close to this design.

| Requirement | Status | Where |
|---|---|---|
| GRD-R1 | Partly built | One Claude Code hook (`hooks/hooks.json`) whose matcher is `Bash\|Write\|Edit`, so `Monitor`, `PowerShell`, `Read`, `Grep`, `Glob` and `NotebookEdit` are not matched yet |
| GRD-R2 | Partly built | Bash walks up to `.planning` (`crates/baley/src/guard/bash.rs:27-44`); Write/Edit has no discovery (`crates/baley/src/guard/mod.rs:116-126`) |
| GRD-R3 | Partly built | For `Bash` (`crates/baley/src/guard/bash.rs:47-151, 456-470`); `Monitor` and `PowerShell` commands are not matched yet (GRD-R1) |
| GRD-R4 | Built | `crates/baley/src/guard/bash.rs:481-486` |
| GRD-R5 | Built | `crates/baley/src/guard/bash.rs:160-252, 369-435` |
| GRD-R6 | Built | `crates/baley/src/guard/bash.rs:369-435` |
| GRD-R7 | Built | `crates/baley/src/guard/audit.rs:150-195` |
| GRD-R8 | Built | `crates/baley/src/guard/audit.rs:9-127` |
| GRD-R9 | Not built | An unrecordable ask becomes allow (`crates/baley/src/guard/bash.rs:444-454`) |
| GRD-R10 | Built | `crates/baley/src/guard/bash.rs:491-501` |
| GRD-R11 | Partly built | Settings files and rendered stubs denied (`crates/baley/src/guard/mod.rs:323-357`); `.planning` paths still listed (`mod.rs:358-385`); no lease check; `NotebookEdit` is not matched yet (GRD-R1) |
| GRD-R12 | Not built | Only Claude Code's form is written, inline in the guard (`crates/baley/src/guard/mod.rs:223-244`). The renderer this requirement describes is built with Build 3 T10 |
| GRD-R13 | Not built | No doctor, and the sandbox settings, the `Read` and `Edit` deny rules and the guard's refusal of a `Read`, `Grep` or `Glob` call are all unbuilt. The sandbox probe is a spike (`spikes/host-matrix`) that Build 3 T12 extends to separate home and config folders and the built-in file tools |
| GRD-R14 | Built | `crates/baley/src/guard/mod.rs:15, 105-129`, `crates/baley/src/guard/bash.rs:254-367` |

## 12. Open questions

No question is open.
