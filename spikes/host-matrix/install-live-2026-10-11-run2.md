# Installed qualification observations

Claude Code version (claude --version): 2.1.296 (Claude Code)
Platform: Linux 7.2.9-zen1-1-zen x86_64
Date of the run: 2026-10-11 (repeat of the install rerun)
Baley commit (pins.txt): b1ec13d57b53477476ccca39e29cc97f21a8143a
Artifact older (artifacts.txt): version 0.0.1 sha256 8b22d5f3f6809813e4bf7dda10d8f86f6def106ec02603d7bf160094604b4135 commit beba9141de2af185b3676bfd2265d77b829ef14d edits Cargo.lock crates/baley/Cargo.toml 
Artifact base (artifacts.txt): version 0.0.2 sha256 88a6c856145735eb3b304a5573e910ffb279f4ced2f91070f86dc932f700aba6 commit b1ec13d57b53477476ccca39e29cc97f21a8143a edits Cargo.lock crates/baley/Cargo.toml 
Artifact newer (artifacts.txt): version 0.1.0 sha256 5498313d585575c30786b20023c92baa5b9c06a0bc843792683240f3b5830f1a commit b1ec13d57b53477476ccca39e29cc97f21a8143a edits crates/baley/src/help/table.rs crates/baley/src/host_artifacts/stubs.rs 
Artifact source, base address (https://raw.githubusercontent.com/<owner>/<repository>/<branch>): https://raw.githubusercontent.com/crenshawdev/baley/live-install-artifacts at commit 5098732e

The procedure is /code/baley/spikes/host-matrix/live-install.md. Fill the mark of every row with pass, fail or unavailable, or observed for a row
that records without an expectation. Actual outcome is what a file, the ledger or a receipt showed
(ls -l, cat, jq, sqlite3 with -readonly, the installer's or baley's printed lines), never only what the model
reported. Evidence is a results file and line, or the pasted output. Class stays blank for the record.
Approve every permission prompt unless a part says otherwise, and note on the row that a prompt appeared.
A host question in a step is one of the eight in the procedure's last section.

## Before the run

The artifact source, the version of Claude Code and the login into the isolated configuration.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| pre.source-older | curl -q --proto '=https' --tlsv1.2 -fsSL <older source>/<os>-<arch>/manifest (host question 8) | Two lines, version 0.0.1 and a 64-digit sha256, each ending in a newline, served over a publicly trusted certificate (design 0012 section 5, Installer and baley install). Host question 8: the throwaway branch serves the artifacts | pass | version 0.0.1, sha256 8b22d5f3...4135, the digest artifacts.txt records | results/p0-source-older.txt | none |
| pre.source-base | The same request for <base source> (host question 8) | Two lines, version 0.0.2 and a 64-digit sha256 (design 0012 section 5, Installer and baley install). Host question 8 | pass | version 0.0.2, sha256 88a6c856...aba6, the digest artifacts.txt records | results/p0-source-base.txt | none |
| pre.source-newer | The same request for <newer source> (host question 8) | Two lines, version 0.1.0 and a 64-digit sha256 (design 0012 section 5, Installer and baley install). Host question 8 | pass | version 0.1.0, sha256 5498313d...0f1a, the digest artifacts.txt records | results/p0-source-newer.txt | none |
| pre.claude-version | claude --version, before any session | Prints a version; recorded in the header | observed | 2.1.296 (Claude Code) | results/claude-version.txt | none |
| pre.login | The login into the isolated configuration, typed by the owner, then test -f ~/.local/share/baley-install-live/claude-config/.credentials.json (presence only) | The file is present, and a session started in the isolated configuration shows no login screen afterwards | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |

## Older server, setup

Part 1. The older binary places its own project before any newer binary has written the per-user records.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| old.installer | sh install.sh <older source>, with the prefix | baley: placed version 0.0.1 at ~/.local/share/baley-install-live/home/.local/bin/baley, then baley: versions present in ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions: with 0.0.1 listed, then the older binary refuses install because it has no such command, and the installer ends non-zero (design 0012 section 5, Installer and baley install: the installer ends with the status of its last step) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| old.init | baley init in project two through ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/0.0.1/baley | Exit status 0 and a baley.toml in project two (baley init --help). It runs before the base install because a binary at view set 7 cannot write the per-user records once a binary at view set 8 has (design 0012 section 5, baley update) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |

## Install

Part 2. The development install script into the disposable HOME, and what a session sees before and after it.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| inst.before.server | Session before-install, started before the install: ask which MCP servers it has, and whether a skill named bal-capture or bal-help is listed | No server named baley and no Baley stub: nothing is installed yet (design 0012 section 5, Installer and baley install) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| inst.script.receipt | sh install.sh <base source>, with the prefix: the installer's own lines | baley: placed version 0.0.2 at ~/.local/share/baley-install-live/home/.local/bin/baley, then baley: versions present in ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions: with 0.0.1 and 0.0.2 listed (design 0012 section 5, Installer and baley install) | pass | baley: placed version 0.0.2 at the stable path; versions present: 0.0.2 only, since Part 1 (the older binary) was not run in this repeat | results/p2-installer.txt lines 1-3 | none |
| inst.script.install-receipt | The receipt baley install prints as the installer's last step | install outcome: complete, baley 0.0.2 at ~/.local/share/baley-install-live/home/.local/bin/baley, one line per artifact (bal-capture, bal-help, the registration, the hook, the settings) and no gap line (design 0012 section 5, Receipt) | pass | install outcome: complete; baley 0.0.2 at the stable path; stub bal-capture written, stub bal-help written, registration registered, hook written, settings written; no gap: line | results/p2-installer.txt lines 4-9 | none |
| inst.script.exit | The installer's exit status, from echo $? right after it | 0: the installer ends with baley install's status, and 0 is complete (design 0012 section 5, Installer and baley install) | pass | installer exit status: 0 | results/p2-installer.txt last line | none |
| inst.register.stored | jq .mcpServers.baley ~/.local/share/baley-install-live/claude-config/.claude.json (host question 3) | command ~/.local/share/baley-install-live/home/.local/bin/baley and args ["serve"], with no alwaysLoad, env or type key (design 0012 section 5, Where each artifact goes). Host question 3: claude mcp add-json --scope user honours CLAUDE_CONFIG_DIR, so this file holds the entry | pass | command the stable path, args ["serve"], no alwaysLoad, env or type key | results/p2-reads.txt == registration | none |
| inst.register.real-unchanged | The owner's real ~/.claude.json: the digest of its mcpServers.baley entry, or absent when the file does not exist, against real-claude-registration-sha256 in pins.txt (host question 3) | Equal: the registration went into the isolated folder and not into the owner's file. Host question 3 | pass | the digest of the real mcpServers.baley entry equals the real-claude-registration-sha256 line of pins.txt (70056b5e...3590) | results/p2-reads.txt == real registration digest | none |
| inst.settings.written | jq on ~/.local/share/baley-install-live/claude-config/settings.json: the PreToolUse items, permissions.deny and sandbox | A Baley PreToolUse item with the nine-tool matcher and a 10-second timeout whose command runs the single-quoted stable path and guard, Baley's Read and Edit deny rules, and its sandbox block with denyRead and denyWrite for the folders (design 0012 section 5, Where each artifact goes and Sandbox and provider access) | pass | one Baley PreToolUse item (nine-tool matcher, the single-quoted stable path and guard, timeout 10) beside the owner item; Read and Edit deny rules for the home and config folders, Edit for the versions folder and the placed files; sandbox denyRead and denyWrite | results/p2-reads.txt == settings | none |
| inst.stubs.written | ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md and ~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md against baley artifact stub <identity> from the base binary | Both exist and equal the base binary's stub bytes (design 0012 section 5, Where each artifact goes) | pass | bal-capture same, bal-help same (artifact stub output of 0.0.2 equals the placed SKILL.md) | results/p2-reads.txt == stubs | none |
| inst.old-session.server | Session before-install, after the install: ask again which MCP servers it has | Still no baley server: a new Claude Code session loads the wiring (design 0012 section 5, Installer and baley install) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| inst.old-session.stub | Session before-install, after the install: ask again which skills it lists (host question 7) | Record whether bal-capture and bal-help appear. Host question 7 asks whether Claude Code finds new stubs only in new sessions | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| inst.init | baley init in project one, through the stable path | Exit status 0 and a baley.toml in project one (baley init --help: it writes baley.toml and records project.initialized) | pass | exit status 0, baley.toml written, project d8e043b2-1d1e-42f3-8c48-6e6c034f9818 created | results/p2-reads.txt == init one | none |
| inst.after.mcp | Session after-install: /mcp | baley is listed from the user scope with its three tools baley_version, baley_query and baley_apply (design 0012 section 5, baley_version, baley_query and baley_apply) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| inst.after.version | Session after-install: call baley_version | The answer names version 0.0.2 (design 0012 section 5, baley_version) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| inst.after.hook | Session after-install: ask for Bash git push origin main, and answer no at the prompt | The installed hook answers: the guard asks (design 0010 GRD-R4), and the user ledger holds a guard.answered with outcome ask for tool Bash whose caller is a hook | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| inst.after.stubs | Session after-install: ask which skills it lists (host question 7) | bal-capture and bal-help are listed: a new session loads the stubs (design 0012 section 5, Installer and baley install). Host question 7 | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |

## Older server, session

Part 3. A server from the T14 merge keeps answering after the stable path moves to a binary at view set 8. Project two was written by the older binary alone, and nothing newer has touched it.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| old.relink-older | sh install.sh <older source> again, with the prefix, after the install | The installer places 0.0.1 from the staged file, lists 0.0.1 and 0.0.2 as present, moves the stable link to ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/0.0.1/baley and ends non-zero because the older binary has no install command (design 0012 section 5, Installer and baley install) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| old.s3.version | Session older-s3, started in project two: call baley_version | The answer names version 0.0.1 | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| old.s3.capture-1 | Session older-s3: baley_apply capture with request id 17170000-0000-4000-8000-000000000021, kind note, text older first | Recorded in project two, with the older server's baley_session in its caller: install writes only user, so the session's own project stays writable for an older server (design 0012 section 5, baley update) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| old.relink-base | sh install.sh <base source> while older-s3 runs | The installer places 0.0.2 from the staged file, moves the stable link to ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/0.0.2/baley and ends with install's status 0 (design 0012 section 5, Installer and baley install) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| old.s3.keeps | Session older-s3, one request after another: baley_version, then baley_apply capture with request id 17170000-0000-4000-8000-000000000022, kind note, text older second, then baley_version | Both baley_version answers name 0.0.1 and the capture is recorded: the older server keeps answering beside the newer stable path (design 0012 section 5, baley update) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| old.s3.same-server | The baley_session of captures 17170000-0000-4000-8000-000000000021 and 17170000-0000-4000-8000-000000000022 in project two | Equal: one server answered both, since the caller records no server version. Unavailable, with the answers the two captures got, when either was refused | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| old.s3.hook | Session older-s3: Bash git push origin main, and answer no at the prompt | The newer hook through the stable path answers: a guard.answered with outcome ask is recorded in user after the relink, which a binary at view set 7 could not write (design 0012 section 5, baley update: the hook follows the stable path) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| old.user-refusal | The older binary's own guard, run by hand with a Bash git push hook call on its standard input (the procedure gives the command) | The guard says the ledger could not record the decision because user is read-only for this binary (the live view set 8 is newer than its 7) and denies the call, since an ask it cannot record becomes a deny (design 0012 section 5, baley update: a binary at view set 7 is read-only on user once a newer binary has brought its views to 8, and design 0010 GRD-R9) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |

## Doctor

Part 4. The doctor runs here and not earlier: it brings every project's views to the current set, which would fence the older server out of project two.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| inst.doctor | baley doctor through the stable path at 0.0.2, with its exit status | A stable path line naming staged version 0.0.2, the install record read, every installed location writable and no finding; exit status 0 (design 0012 section 5, baley doctor) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |

## Protection

Part 5. Four protected places and seven tools each, then the stable path's staged file, a bypass-permissions session and the network requests.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| prot.home.bash-read | Bash read: cat ~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley/seed.txt | Refused by the sandbox (denyRead) and the file appears missing (design 0012 section 5, Sandbox and provider access, and design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.home.bash-write | Bash write: echo bash > ~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley/agent-bash.txt | Refused by the sandbox (denyWrite) and nothing lands in the folder (design 0012 section 5, Sandbox and provider access, and design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.home.read | Read tool: ~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley/seed.txt | Refused by the Read deny rule (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.home.write | Write tool: create ~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley/agent-write.txt | Refused by the Edit deny rule (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.home.edit | Edit tool: change 'FAKE_KEY' to 'EDITED_KEY' in ~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley/seed.txt | Refused by the Edit deny rule (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.home.grep | Grep tool (search-tools session): pattern FAKE_KEY, path ~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley | Refused by the guard, which denies a Grep whose target lies inside the folder or holds it. Claude Code applies the Read rule to Grep only on a best-effort basis (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.home.glob | Glob tool (search-tools session): pattern *, path ~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley | Refused by the guard, which denies a Glob whose target lies inside the folder or holds it. Claude Code applies the Read rule to Glob only on a best-effort basis (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.config.bash-read | Bash read: cat ~/.local/share/baley-install-live/home/.config/crenshawdev/baley/seed.txt | Refused by the sandbox (denyRead) and the file appears missing (design 0012 section 5, Sandbox and provider access, and design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.config.bash-write | Bash write: echo bash > ~/.local/share/baley-install-live/home/.config/crenshawdev/baley/agent-bash.txt | Refused by the sandbox (denyWrite) and nothing lands in the folder (design 0012 section 5, Sandbox and provider access, and design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.config.read | Read tool: ~/.local/share/baley-install-live/home/.config/crenshawdev/baley/seed.txt | Refused by the Read deny rule (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.config.write | Write tool: create ~/.local/share/baley-install-live/home/.config/crenshawdev/baley/agent-write.txt | Refused by the Edit deny rule (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.config.edit | Edit tool: change 'FAKE_KEY' to 'EDITED_KEY' in ~/.local/share/baley-install-live/home/.config/crenshawdev/baley/seed.txt | Refused by the Edit deny rule (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.config.grep | Grep tool (search-tools session): pattern FAKE_KEY, path ~/.local/share/baley-install-live/home/.config/crenshawdev/baley | Refused by the guard, which denies a Grep whose target lies inside the folder or holds it. Claude Code applies the Read rule to Grep only on a best-effort basis (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.config.glob | Glob tool (search-tools session): pattern *, path ~/.local/share/baley-install-live/home/.config/crenshawdev/baley | Refused by the guard, which denies a Glob whose target lies inside the folder or holds it. Claude Code applies the Read rule to Glob only on a best-effort basis (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.versions.bash-read | Bash read: cat ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/seed.txt | Allowed: the proposal gives the versions folder no denyRead entry and no Read rule, so sandboxed commands can run the binaries inside it (design 0012 section 5, Sandbox and provider access) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.versions.bash-write | Bash write: echo bash > ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/agent-bash.txt | Refused by the sandbox (denyWrite names the folder and the stable path) and nothing lands (design 0012 section 5, Sandbox and provider access). | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.versions.read | Read tool: ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/seed.txt | Allowed: no Read rule covers it (design 0012 section 5, Sandbox and provider access) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.versions.write | Write tool: create ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/agent-write.txt | Refused by the Edit rule for the folder or the stable path (design 0012 section 5, Sandbox and provider access). | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.versions.edit | Edit tool: change 'FAKE_KEY' to 'EDITED_KEY' in ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/seed.txt | Refused by the Edit rule for the folder or the stable path (design 0012 section 5, Sandbox and provider access). | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.versions.grep | Grep tool (search-tools session): pattern FAKE_KEY, path ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions | Allowed: the guard refuses a Grep only for Baley's home and config folders (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.versions.glob | Glob tool (search-tools session): pattern *, path ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions | Allowed: the guard refuses a Glob only for Baley's home and config folders (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.stable.bash-read | Bash read: ~/.local/share/baley-install-live/home/.local/bin/baley --version, which runs the staged binary through the link | Allowed: the stable path links into the versions folder, which has no denyRead entry (design 0012 section 5, Sandbox and provider access) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.stable.bash-write | Bash write: echo x >> ~/.local/share/baley-install-live/home/.local/bin/baley (host question 2) | Refused by the sandbox (denyWrite names the folder and the stable path) and nothing lands (design 0012 section 5, Sandbox and provider access). Host question 2: the staged file behind the link is not changed | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.stable.read | Read tool: ~/.local/share/baley-install-live/home/.local/bin/baley | Allowed: no Read rule covers it (design 0012 section 5, Sandbox and provider access) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.stable.write | Write tool: replace ~/.local/share/baley-install-live/home/.local/bin/baley with one line (host question 2) | Refused by the Edit rule for the folder or the stable path (design 0012 section 5, Sandbox and provider access). Host question 2: the staged file behind the link is not changed | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.stable.edit | Edit tool: change 'a' to 'b' in ~/.local/share/baley-install-live/home/.local/bin/baley (host question 2) | Refused by the Edit rule for the folder or the stable path (design 0012 section 5, Sandbox and provider access). Host question 2: the staged file behind the link is not changed | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.stable.grep | Grep tool (search-tools session): pattern baley, path ~/.local/share/baley-install-live/home/.local/bin/baley | Allowed: the guard refuses a Grep only for Baley's home and config folders (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.stable.glob | Glob tool (search-tools session): pattern baley, path ~/.local/share/baley-install-live/home/.local/bin | Allowed: the guard refuses a Glob only for Baley's home and config folders (design 0010 GRD-R13) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.stable.staged-intact | sha256sum ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/0.0.2/baley after the rows above, against the sha256 line of the base manifest (host question 2) | Equal: nothing wrote the staged binary through the link (design 0012 section 5, Sandbox and provider access). Host question 2 | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.versions.no-stray | ls -l ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions after the rows above | No agent-bash.txt or agent-write.txt and no change to seed.txt: nothing landed (design 0012 section 5, Sandbox and provider access) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.bypass.read-home | Session bypass (bypassPermissions): Read tool on ~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley/seed.txt (host question 1) | Refused by the Read deny rule, which still applies in this mode (design 0010 GRD-R13). Host question 1: this row settles the deny rule, not the hook | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.bypass.bash-write-home | Session bypass: Bash write: echo bash > ~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley/agent-bypass.txt (host question 1) | Refused by the sandbox, which still applies in this mode (design 0010 GRD-R13). Host question 1: this row settles the sandbox, not the hook | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.bypass.hook-push | Session bypass: Bash git push origin main, answer no at any prompt, then the user ledger's guard.answered events against the time written down before the request (host question 1) | The hook still runs: a new guard.answered with outcome ask, tool Bash and a hook caller, recorded after that time, since no deny rule or sandbox entry covers this command (design 0010 GRD-R4). No such event means the hook does not run in this mode. Host question 1: this row settles the hook | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.net.user-listed | Session protect: Bash curl -sSI https://example.com, a host the user settings file lists | Record whether a prompt appeared and whether the request went through | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.net.unlisted | Session protect: Bash curl -sSI https://example.org, a host no settings file lists, while the user file lists one owner host (host question 4) | Record whether the request is cut off with no prompt, asks, or goes through. Design 0012 section 5 (Sandbox and provider access) says install keeps every host the file holds and adds none; host question 4 asks whether a present list only pre-approves its hosts | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.net.project-owner-host | Session network-project, with project one's own .claude/settings.json listing www.iana.org: Bash curl -sSI https://example.com, the owner host the user file lists (host question 5) | Record whether a prompt appeared. Host question 5 asks whether a project's list adds to the user file's list | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| prot.net.project-own-host | Session network-project: Bash curl -sSI https://www.iana.org, the host only project one's file lists (host question 5) | Record whether a prompt appeared. Host question 5 | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |

## Opt-in update

Part 6. updates.auto on, a new session, and a newer version staged without the session waiting.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| optin.set | baley config set --global updates.source=<newer source>, then baley config set --global updates.auto=true, then baley config show | Both commands exit 0 and config show lists updates.source and updates.auto true as global settings (design 0012 section 5, baley update) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| optin.session | Session opt-in, started after both settings: call baley_version | The answer names version 0.0.2: the server started on the version the stable path ran then, and activation does not touch a running server (design 0012 section 5, baley update) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| optin.staged | The user ledger's update.checked events and the stable link, after the session started | One update.checked with actor baley, outcome staged and active_version and staged_version 0.1.0, and the stable link now targets ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/0.1.0/baley (design 0012 section 6, Update checks) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| optin.no-wait | Session opt-in's debug log: the Starting connection time and the Successfully connected figure, the first baley_version Calling time and its completed figure, and the staged binary's modification time, with the same two figures from session after-install | Neither the connection nor the served call waited for the download: both took about as long as the after-install figures. Fail when either took markedly longer and began before the staged binary's modification time and ended at or after it. Unavailable, with the reason, when the download finished before the Starting connection line, or when either took markedly longer without spanning the end of the download. Which came first, the answer or the end of the download, is written down and does not decide the row (design 0012 section 5, baley update: the server starts a detached process without a wait) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |

## Manual update

Part 7. updates.auto off, the base version back behind the stable path, and baley update while a session runs.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| man.setting-off | baley config set --global updates.auto=false | Exit 0 (design 0012 section 5, baley update) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| man.relink | sh install.sh <base source> | The installer places 0.0.2 again, prints versions present listing 0.0.1, 0.0.2 and 0.1.0, and the stable link targets ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/0.0.2/baley (design 0012 section 5, Installer and baley install: it moves the link and removes no version) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| man.s1.version | Session manual-s1: call baley_version | The answer names version 0.0.2 | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| man.update.receipt | baley update while manual-s1 runs, with its exit status | update outcome: staged, staged version: 0.1.0, 0.1.0 is now the active version at ~/.local/share/baley-install-live/home/.local/bin/baley, new Claude Code sessions start 0.1.0; running sessions keep the version they started with, the catalog seed line and a kept versions line; exit status 0 (design 0012 section 5, baley update, Receipt) | pass | run with no session open, after updates.source was set to the newer source: update outcome: staged; staged version: 0.1.0; 0.1.0 is now the active version at the stable path; new sessions start 0.1.0; catalog seed recorded; kept versions: 0.0.2, 0.1.0; exit status 0 | results/p7-update.txt == update | none |
| man.s1.keeps | Session manual-s1 again: call baley_version | Still 0.0.2: a running session keeps the version it started with (design 0012 section 5, baley update) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| man.s2.new | Session manual-s2, started after the update: call baley_version | The answer names version 0.1.0 (design 0012 section 5, baley update) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| man.ledger | The user ledger's update.checked events | A second update.checked, with actor owner and outcome staged, after the opt-in one with actor baley. A manual check never refuses a day already claimed (design 0012 section 6, Daily claim) | observed | one update.checked, actor owner, outcome staged, active and staged 0.1.0 (seq 6). There is no opt-in check before it, since Part 6 was not run in this repeat | results/p7-update.txt == events | note |

## Install rerun

Part 8. baley install through the stable path at the newer version. The newer artifact changes the bal-capture stub's text and drops bal-help from its manifest.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| rerun.doctor-stale | Before the rerun, baley doctor through the stable path at 0.1.0, with its exit status | A line for bal-help reading stale, since this binary no longer ships it, with baley install as its fix, and exit status 1 (design 0012 section 5, baley doctor, and HST-R12: the doctor checks the stubs against the version they serve) | pass | stub bal-help: is stale, since this binary no longer ships it; run baley install so the install record stops listing it; doctor exit status 1 | results/doctor-before-rerun.txt lines 51, 86 | none |
| rerun.doctor-capture | The same doctor output: its line for bal-capture | Recorded as seen: the placed stub carries the base text and the newer manifest differs (design 0012 section 5, baley doctor, which compares each placed stub with the manifest) | pass | stub bal-capture: is an older stub Baley placed, not the one this version renders: expected 99c85d7c...9ba3, found 1d0e290f...4bf7; run baley install to bring it to this version (the wording d95f482a gave it) | results/doctor-before-rerun.txt line 47 | none |
| rerun.stub-sums-before | sha256sum of ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md and ~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md, before the rerun | The base manifest's digests for both | observed | bal-capture 1d0e290f...4bf7, bal-help 1be9fec2...e1a9, the base manifest digests | results/p8-stub-sums-before.txt | none |
| rerun.receipt | baley install through the stable path, with its exit status | install outcome: complete and exit status 0, with the bal-capture line reading replaced, the registration line reading unchanged (it is this binary's entry, so no claude command runs) and no line for bal-help, which the newer manifest no longer ships. The hook and settings lines are recorded as seen (design 0012 section 5, Receipt, and Ownership: a stub whose SHA-256 is the hash the latest record holds is Baley's and is replaced) | pass | install outcome: complete, exit status 0; stub bal-capture: replaced; registration: unchanged; hook: written; settings: written; stub bal-help: removed, since this binary no longer ships it | results/p8-rerun.txt lines 1-7, last line | none |
| rerun.record | The latest install.recorded in the user ledger | binary_version 0.1.0 and complete true (design 0012 section 6, install.recorded) | pass | install.recorded seq 8: binary_version 0.1.0, complete 1, stubs bal-capture only (99c85d7c...9ba3) | results/p8-reads.txt == record (step 4) | none |
| rerun.capture-upgraded | sha256sum of ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md after the rerun, against the bal-capture digest of newer's manifest in artifacts.txt | Changed from the digest taken before the rerun and equal to newer's manifest digest (design 0012 section 5, Ownership) | pass | 99c85d7c...9ba3, differs from the step 2 digest and equals the stub newer bal-capture line of artifacts.txt | results/p8-reads.txt == capture digest (step 5) | none |
| rerun.dropped-stub-file | Whether ~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md is still on disk after baley install | Recorded as seen: the design states nothing about removing a stub file the manifest no longer ships | pass | bal-help/SKILL.md is gone (No such file or directory); its empty bal-help folder is left under skills, as design 0012 says (#259, PR #261) | results/p8-reads.txt == bal-help file (step 6) | none |
| rerun.settings-diff | A diff of the settings file before and after the rerun, each side sorted with jq -S | Recorded as seen: each entry added or removed. Design 0012 section 5 (Ownership) says an older Baley entry leaves only on the record's evidence and the owner's entries stay | observed | Removed: the Edit rule and the denyWrite entry for bal-help/SKILL.md, whose file the rerun removed. Nothing added | results/p8-reads.txt == settings diff (step 6) | none |
| rerun.one-hook | The number of Baley PreToolUse items in ~/.local/share/baley-install-live/claude-config/settings.json | Exactly one (design 0012 section 5, Installer and baley install: running install again adds only entries that are absent) | pass | 1 | results/p8-reads.txt == one hook (step 7) | none |
| rerun.one-registration | jq on ~/.local/share/baley-install-live/claude-config/.claude.json: the keys of mcpServers | One baley entry, equal to the one stored before (design 0012 section 5, Installer and baley install) | pass | ["baley"], and the entry is the same as registration-before.json | results/p8-reads.txt == one registration (step 7) | none |
| rerun.owner-key | The seeded owner key cleanupPeriodDays in ~/.local/share/baley-install-live/claude-config/settings.json | Kept with its value 30 (design 0012 section 5, Installer and baley install: existing owner settings are preserved) | pass | cleanupPeriodDays 30 | results/p8-reads.txt == owner settings (step 8) | none |
| rerun.owner-deny | The seeded owner permissions.deny rule over ~/.local/share/baley-install-live/owner-private | Kept beside Baley's rules | pass | Read(owner-private/**) kept | results/p8-reads.txt == owner settings (step 8) | none |
| rerun.owner-hook | The seeded owner PreToolUse item with its own matcher | Kept beside Baley's item | pass | the Bash\|WebFetch item running true kept | results/p8-reads.txt == owner settings (step 8) | none |
| rerun.owner-host | The seeded owner host example.com in sandbox.network.allowedDomains | Kept (design 0012 section 5, Sandbox and provider access: install keeps every host the file already holds) | pass | ["example.com"] | results/p8-reads.txt == owner settings (step 8) | none |

## In-flight exit

Part 9. Claude Code's process ended while Baley calls run, with the three tools pre-allowed so no prompt holds a call back.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| exit.r1.signal | Round 1 (session inflight-1): SIGTERM sent to Claude Code's process on the first baley_apply call line of its debug log; the time sent and the request ids the prompt gave | Recorded as seen | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| exit.r1.in-flight | Round 1: capture.recorded events whose recorded_at is later than the time the signal was sent, and the server's own lines where they appear (host question 6) | The server held at least one call when the signal came: a capture.recorded later than the signal, or its own work-still-open line. A call line without a completed line in Claude Code's log is the client's view and does not count by itself. Mark unavailable with the reason when neither server record exists, since the client log alone cannot show whether a call was inside the server | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| exit.r1.answered | Round 1: the ledgers for each request id the prompt gave, the server's own lines and the replies in the transcript | Every id the session sent is in the ledger once and none twice, no server line reports open work, and no reply with a receipt names an id the ledger lacks, which fails the row (design 0012 section 7, exit, and ADR 0034). An id the session sent that the ledger lacks, with no such reply, keeps the row from passing: unavailable for that id, the ids written on the row, since the client log alone cannot show whether the server read the call and dropped it in shutdown. A refusal the server recorded for the id counts as its answer, and the server's own work-still-open line fails the row | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| exit.r1.server-end | Round 1: how the debug log shows the server ending (a signal Claude Code sent it, or its input closing) (host question 6) | Recorded as seen. Host question 6 asks whether Claude Code ends a stdio server's input while a call runs when a session ends | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| exit.r1.stderr | Round 1: where the server's exit checkpoint line and any drain line appear without a redirect | Recorded as seen: the earlier live runs never ran the stderr control, so this is not yet known | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| exit.r2.signal | Round 2 (session inflight-2): SIGKILL sent the same way, run only when round 1 showed no call in flight; the time sent and the request ids | Recorded as seen | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| exit.r2.in-flight | Round 2: capture.recorded events later than the signal's time, and the server's own lines where they appear | The server held at least one call when the signal came, as for round 1, else unavailable with the reason (host question 6) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| exit.r2.answered | Round 2: the ledgers for each request id the prompt gave, the server's own lines and the replies in the transcript | As for round 1: every id the session sent is in the ledger once, no server line reports open work, and no reply with a receipt names an id the ledger lacks, which fails the row. A sent id missing from the ledger with no such reply keeps the row from passing: unavailable for that id, the ids written, unless a refusal the server recorded accounts for it (design 0012 section 7, exit, and ADR 0034) | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| exit.condition | Whether any round put a call inside the server when Claude Code's process ended, as the ledger and the server's own lines show it (host question 6) | Recorded as seen: when no round shows the server holding a call, the exit rows read unavailable and the reason is written here | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |

## Post-run

Part 10. After every session has exited: sh live-install-reads.sh > ~/.local/share/baley-install-live/results/reads.txt.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| post.verify-user | baley verify --local-only user | Exit status 0 | pass | exit status 0 | results/reads.txt line 160 | none |
| post.views-user | baley verify --views user | Exit status 0 | pass | exit status 0 | results/reads.txt line 165 | none |
| post.verify-one | baley verify --local-only for project one | Exit status 0 | pass | exit status 0 | results/reads.txt line 176 | none |
| post.views-one | baley verify --views for project one | Exit status 0 | pass | exit status 0 | results/reads.txt line 181 | none |
| post.verify-two | baley verify --local-only for project two | Exit status 0 | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| post.views-two | baley verify --views for project two | Exit status 0 | unavailable | not run in this repeat: it covers the base install, a manual update and the install rerun (Part 8), in the shell with no Claude Code session | - | unverified |
| post.doctor | baley doctor through the stable path, with its exit status | Exit status 0: the rerun's record no longer lists bal-help, so nothing is stale (design 0012 section 5, baley doctor) | pass | exit status 0; nothing stale after the rerun | results/reads.txt lines 186-271 | none |
| post.owner-settings | The seeded owner key, deny rule, PreToolUse item and host against results/owner-settings.json | Each marked kept | pass | all four kept | results/reads.txt lines 303-307 | none |
| post.real-installation | The owner's real ~/.local/bin/baley, ~/.claude/settings.json, mcpServers.baley digest and the real Baley folders against pins.txt | No difference | pass | no difference for ~/.local/bin/baley, ~/.claude/settings.json, the mcpServers.baley entry, the claude launcher and both real Baley folders | results/reads.txt lines 318-321 | none |
| post.real-claude | The owner's claude command and ~/.local/bin/claude link against pins.txt | No difference, and neither resolves inside either disposable root | pass | the claude command and the launcher resolve outside both disposable roots (~/.local/share/claude/versions/2.1.296) | results/reads.txt lines 323-325 | none |

## About this record

This is the sheet of a repeat of part of the installed qualification run of 2026-10-11, whose sheet is `install-live-2026-10-11-run1.md`. It was made after three fixes reached `main` and this branch, to check the rows of the first sheet that those fixes change. It is a shell-only repeat: no Claude Code session was started and no login was typed. The Baley commit in the header, b1ec13d57b53477476ccca39e29cc97f21a8143a, is this branch after `main` was merged into it, so it holds the fixes. The development artifacts were built at that commit and published as commit 5098732e1d4f29c2737532d9da9810e98abfed4b on the throwaway branch `live-install-artifacts` of `crenshawdev/baley`, served through `raw.githubusercontent.com`. All three manifests served the digests `artifacts.txt` records. The first run's disposable tree and artifacts were moved aside before the repeat, so the script cleared nothing of the first run. A row is cited as `install-live-2026-10-11-run2.md#<row id>`.

### What ran

The repeat ran, from `live-install.md`: Part 2 steps 2, 3 and 5 (the base version 0.0.2 by `install.sh`, its reads, and `baley init` in project one), Part 6 step 1 with its first command only (`updates.source` set to the newer source, `updates.auto` left off), Part 7 step 4 (`baley update` with no session open, which put 0.1.0 at the stable path), Part 8 whole (the install rerun) and Part 10 step 1 (the reads). Part 1 was not run, so 0.0.1 is not staged and project two is not initialised. The sheet holds 112 rows: 32 pass, 4 observed, 76 unavailable and 0 fail. Every row marked unavailable was not run, and its outcome cell says so.

Two things were done to the sheet after the repeat. The class column, which the repeat left blank as its own instructions say, was filled in from the rules below, and one `|` inside the outcome cell of `rerun.owner-hook` was written `\|` so the table keeps its columns. No mark, no outcome text and no evidence cell was changed otherwise. The section "Cited evidence" holds the lines of the result files that the evidence cells cite, and the last section holds the post-run reads unchanged.

### Classes

The sheet reads 35 none, 1 note, 76 unverified and 0 defect. The rules are the first sheet's.

- `none`: a row that passed, or an observed row that carries no finding.
- `note`: an observed row that carries a finding.
- `unverified`: the row could not be made. A row marked unavailable always reads unverified.
- `defect`, `expected`, `host limit` and `unclear` name no row of this repeat. No row failed.

The one row classed note is `man.ledger`: it expects a second `update.checked` after an opt-in one, and with Part 6 not run the ledger holds one event, the manual one, with actor `owner` and outcome `staged`.

### Rows of the first sheet that this repeat answers

The first sheet is kept as committed, and it still reads note for three rows. Read against the fixes below, `rerun.doctor-capture`, `rerun.dropped-stub-file` and `rerun.settings-diff` of the first sheet are defects in earlier production code, and `old.s3.capture-1` and `old.s3.keeps` are the defect that sheet already classed. Four rows of the first sheet stay note: `inst.old-session.stub`, `exit.r1.server-end`, `exit.r1.stderr` and `exit.condition`. None of them is a fixed defect and all of them need a Claude Code session, so none was repeated.

| First-sheet row | Class on the first sheet | Class read against the fix | Fix | What the repeat shows |
|---|---|---|---|---|
| `rerun.doctor-capture` | note | defect | commit d95f482a, which names no issue | `rerun.doctor-capture` passes. The doctor's line for the older `bal-capture` stub reads "is an older stub Baley placed, not the one this version renders" and its fix is "run baley install to bring it to this version". `rerun.receipt` shows install replaced the stub with no step to move it aside. |
| `rerun.dropped-stub-file` | note | defect | issue #259, fixed by PR #261 (merged to `main` as 28f79d21) | `rerun.dropped-stub-file` passes. Install removed `bal-help/SKILL.md` and left the empty `skills/bal-help` folder, as design 0012 says. |
| `rerun.settings-diff` | note | defect | issue #259, fixed by PR #261 | `rerun.settings-diff` is observed. The `Edit` rule and the `denyWrite` entry for the retired stub left the settings file together with its file, and nothing was added. |
| `old.s3.capture-1`, `old.s3.keeps` | defect | defect | issue #258, fixed by PR #260 (merged to `main` as a0d66781) | Not repeated. See the next section. |

### Older-server rows

Issue #258, fixed by PR #260, makes the policy step take catalog version 0 when its read of `user` is refused for a newer view set, so an older server can still write its own project (`crates/baley/src/policy_step/mod.rs:48-63`). Design 0012 section 5, in the `baley update` paragraph, now says that a binary built before the fix can write no project once `user` is at a newer view set.

The live check of the fix needs an older binary that already holds it, running beside a newer binary that raises the view set again. No such pair exists: the older artifact of the first run is built before the fix, and nothing since has raised the view set. So these rows were not run: `old.installer`, `old.init`, `old.relink-older`, `old.s3.version`, `old.s3.capture-1`, `old.relink-base`, `old.s3.keeps`, `old.s3.same-server`, `old.s3.hook` and `old.user-refusal`. They read unverified, and the first sheet's two defect rows stay the record of what the older server did.

The repeat is owed at the next raise of the view set. It belongs to the task that makes that raise, at the registered view set (`ViewSet` in `crates/baley-store-sqlite/src/view.rs`), and it runs the older-server rows with an older binary built from `main` at or after a0d66781 beside a newer binary at the raised set.

### Findings

These are recorded as found. No code changed for them and none has a ruling yet.

1. The expected text of `rerun.dropped-stub-file` and `rerun.settings-diff` in `live-install.md` and in the row text that `live-install.sh` writes says the design states nothing about removing a stub file the manifest no longer ships. Since #259 it does, in design 0012 section 5 under Ownership. The rows are marked against the design as it stands. The expected text in `live-install.md` is corrected in the commit that adds this sheet. The same text in `live-install.sh` was corrected after the repeat, together with the receipt's expected text (see "Rules changed after the repeat").
2. The doctor's stale line for `bal-help` still reads "run baley install so the install record stops listing it". Since #259 install also removes the file, so the line is true and no longer says everything the fix does.
3. The rerun leaves the empty `skills/bal-help` folder, as design 0012 and PR #261 say.

### Rules changed after the repeat

Part 8 step 6 of `live-install.md` now says the retired stub's file is expected to be gone, with its empty folder left, as design 0012 states. The rows above were judged against the design as it stands and are recorded as marked.

After the repeat, the expected text of `rerun.receipt` was corrected in `live-install.md` Part 8 step 3 and in `live-install.sh`. It had said install prints no line for `bal-help`. Since #259 install prints ``stub `bal-help` at <path>: removed, since this binary no longer ships it``, and the row above was marked pass against that removed line, not against the expected text as it stood. The row is recorded as marked. The row text `live-install.sh` writes for `rerun.dropped-stub-file` and `rerun.settings-diff` was corrected in the same change to match Part 8 step 6. The Part 5 heading in `live-install.sh` now says seven checks for each protected place, since Bash is one tool and not two. Those two rows' expected text and the Protection section's heading ("seven tools each") in this sheet stay as the repeat wrote them.

## Cited evidence

Each heading is a row id. Under it are the lines of the repeat's result files that the row's evidence cell cites. A cell that names a file without lines is pasted whole, and a cell that names a section is pasted from its `==` line to the next. The file names are those of the repeat's results folder, which is not kept. A row that was not run cites nothing.

### <a id="pre.source-older"></a>pre.source-older

`results/p0-source-older.txt`:

```
version 0.0.1
sha256 8b22d5f3f6809813e4bf7dda10d8f86f6def106ec02603d7bf160094604b4135
exit 0
```

### <a id="pre.source-base"></a>pre.source-base

`results/p0-source-base.txt`:

```
version 0.0.2
sha256 88a6c856145735eb3b304a5573e910ffb279f4ced2f91070f86dc932f700aba6
exit 0
```

### <a id="pre.source-newer"></a>pre.source-newer

`results/p0-source-newer.txt`:

```
version 0.1.0
sha256 5498313d585575c30786b20023c92baa5b9c06a0bc843792683240f3b5830f1a
exit 0
```

### <a id="pre.claude-version"></a>pre.claude-version

`results/claude-version.txt`:

```
2.1.296 (Claude Code)
```

### <a id="pre.login"></a>pre.login

No result file is cited: the row was not run in this repeat.

### <a id="old.installer"></a>old.installer

Not run in this repeat. The repeat needs an older binary that already holds the fix for issue #258 (merged as PR #260) beside a newer binary that raises the view set again, and no such pair exists. See "Older-server rows" above.

### <a id="old.init"></a>old.init

Not run in this repeat. The repeat needs an older binary that already holds the fix for issue #258 (merged as PR #260) beside a newer binary that raises the view set again, and no such pair exists. See "Older-server rows" above.

### <a id="inst.before.server"></a>inst.before.server

No result file is cited: the row was not run in this repeat.

### <a id="inst.script.receipt"></a>inst.script.receipt

`results/p2-installer.txt`, lines 1-3:

```
baley: placed version 0.0.2 at ~/.local/share/baley-install-live/home/.local/bin/baley
baley: versions present in ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions:
  0.0.2
```

### <a id="inst.script.install-receipt"></a>inst.script.install-receipt

`results/p2-installer.txt`, lines 4-9:

```
install outcome: complete
baley 0.0.2 at ~/.local/share/baley-install-live/home/.local/bin/baley
stub `bal-capture` at ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md: written
stub `bal-help` at ~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md: written
registration at ~/.local/share/baley-install-live/claude-config/.claude.json: registered
hook at ~/.local/share/baley-install-live/claude-config/settings.json: written
```

### <a id="inst.script.exit"></a>inst.script.exit

`results/p2-installer.txt`, the last line:

```
installer exit status: 0
```

### <a id="inst.register.stored"></a>inst.register.stored

`results/p2-reads.txt`, section `registration`:

```
== registration
{
  "command": "~/.local/share/baley-install-live/home/.local/bin/baley",
  "args": [
    "serve"
  ]
}
```

### <a id="inst.register.real-unchanged"></a>inst.register.real-unchanged

`results/p2-reads.txt`, section `real registration digest`:

```
== real registration digest
70056b5ee93a7f08f583971157cf1ea50b8f27cbb89371e308a2a7b984b43590
real-claude-registration-sha256: 70056b5ee93a7f08f583971157cf1ea50b8f27cbb89371e308a2a7b984b43590
```

### <a id="inst.settings.written"></a>inst.settings.written

`results/p2-reads.txt`, section `settings`:

```
== settings
[{"matcher":"Bash|WebFetch","hooks":[{"type":"command","command":"true"}]},{"matcher":"Bash|Monitor|PowerShell|Read|Grep|Glob|Write|Edit|NotebookEdit","hooks":[{"type":"command","command":"'~/.local/share/baley-install-live/home/.local/bin/baley' guard","timeout":10}]}]
["Read(/~/.local/share/baley-install-live/owner-private/**)","Read(/~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley/**)","Edit(/~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley/**)","Read(/~/.local/share/baley-install-live/home/.config/crenshawdev/baley/**)","Edit(/~/.local/share/baley-install-live/home/.config/crenshawdev/baley/**)","Edit(/~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/**)","Edit(/~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md)","Edit(/~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md)","Edit(/~/.local/share/baley-install-live/claude-config/.claude.json)","Edit(/~/.local/share/baley-install-live/claude-config/settings.json)","Edit(/~/.local/share/baley-install-live/home/.local/bin/baley)"]
{"denyRead":["~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley","~/.local/share/baley-install-live/home/.config/crenshawdev/baley"],"denyWrite":["~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley","~/.local/share/baley-install-live/home/.config/crenshawdev/baley","~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions","~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md","~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md","~/.local/share/baley-install-live/claude-config/.claude.json","~/.local/share/baley-install-live/claude-config/settings.json","~/.local/share/baley-install-live/home/.local/bin/baley"]}
```

### <a id="inst.stubs.written"></a>inst.stubs.written

`results/p2-reads.txt`, section `stubs`:

```
== stubs
bal-capture same
bal-help same
```

### <a id="inst.old-session.server"></a>inst.old-session.server

No result file is cited: the row was not run in this repeat.

### <a id="inst.old-session.stub"></a>inst.old-session.stub

No result file is cited: the row was not run in this repeat.

### <a id="inst.init"></a>inst.init

`results/p2-reads.txt`, section `init one`:

```
== init one
wrote ~/.local/share/baley-install-live/projects/one/baley.toml (commit this file)
created project d8e043b2-1d1e-42f3-8c48-6e6c034f9818 in the ledger at ~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley
recorded project.initialized for project d8e043b2-1d1e-42f3-8c48-6e6c034f9818
exit status: 0
baley.toml
```

### <a id="inst.after.mcp"></a>inst.after.mcp

No result file is cited: the row was not run in this repeat.

### <a id="inst.after.version"></a>inst.after.version

No result file is cited: the row was not run in this repeat.

### <a id="inst.after.hook"></a>inst.after.hook

No result file is cited: the row was not run in this repeat.

### <a id="inst.after.stubs"></a>inst.after.stubs

No result file is cited: the row was not run in this repeat.

### <a id="old.relink-older"></a>old.relink-older

Not run in this repeat. The repeat needs an older binary that already holds the fix for issue #258 (merged as PR #260) beside a newer binary that raises the view set again, and no such pair exists. See "Older-server rows" above.

### <a id="old.s3.version"></a>old.s3.version

Not run in this repeat. The repeat needs an older binary that already holds the fix for issue #258 (merged as PR #260) beside a newer binary that raises the view set again, and no such pair exists. See "Older-server rows" above.

### <a id="old.s3.capture-1"></a>old.s3.capture-1

Not run in this repeat. The repeat needs an older binary that already holds the fix for issue #258 (merged as PR #260) beside a newer binary that raises the view set again, and no such pair exists. See "Older-server rows" above.

### <a id="old.relink-base"></a>old.relink-base

Not run in this repeat. The repeat needs an older binary that already holds the fix for issue #258 (merged as PR #260) beside a newer binary that raises the view set again, and no such pair exists. See "Older-server rows" above.

### <a id="old.s3.keeps"></a>old.s3.keeps

Not run in this repeat. The repeat needs an older binary that already holds the fix for issue #258 (merged as PR #260) beside a newer binary that raises the view set again, and no such pair exists. See "Older-server rows" above.

### <a id="old.s3.same-server"></a>old.s3.same-server

Not run in this repeat. The repeat needs an older binary that already holds the fix for issue #258 (merged as PR #260) beside a newer binary that raises the view set again, and no such pair exists. See "Older-server rows" above.

### <a id="old.s3.hook"></a>old.s3.hook

Not run in this repeat. The repeat needs an older binary that already holds the fix for issue #258 (merged as PR #260) beside a newer binary that raises the view set again, and no such pair exists. See "Older-server rows" above.

### <a id="old.user-refusal"></a>old.user-refusal

Not run in this repeat. The repeat needs an older binary that already holds the fix for issue #258 (merged as PR #260) beside a newer binary that raises the view set again, and no such pair exists. See "Older-server rows" above.

### <a id="inst.doctor"></a>inst.doctor

No result file is cited: the row was not run in this repeat.

### <a id="prot.home.bash-read"></a>prot.home.bash-read

No result file is cited: the row was not run in this repeat.

### <a id="prot.home.bash-write"></a>prot.home.bash-write

No result file is cited: the row was not run in this repeat.

### <a id="prot.home.read"></a>prot.home.read

No result file is cited: the row was not run in this repeat.

### <a id="prot.home.write"></a>prot.home.write

No result file is cited: the row was not run in this repeat.

### <a id="prot.home.edit"></a>prot.home.edit

No result file is cited: the row was not run in this repeat.

### <a id="prot.home.grep"></a>prot.home.grep

No result file is cited: the row was not run in this repeat.

### <a id="prot.home.glob"></a>prot.home.glob

No result file is cited: the row was not run in this repeat.

### <a id="prot.config.bash-read"></a>prot.config.bash-read

No result file is cited: the row was not run in this repeat.

### <a id="prot.config.bash-write"></a>prot.config.bash-write

No result file is cited: the row was not run in this repeat.

### <a id="prot.config.read"></a>prot.config.read

No result file is cited: the row was not run in this repeat.

### <a id="prot.config.write"></a>prot.config.write

No result file is cited: the row was not run in this repeat.

### <a id="prot.config.edit"></a>prot.config.edit

No result file is cited: the row was not run in this repeat.

### <a id="prot.config.grep"></a>prot.config.grep

No result file is cited: the row was not run in this repeat.

### <a id="prot.config.glob"></a>prot.config.glob

No result file is cited: the row was not run in this repeat.

### <a id="prot.versions.bash-read"></a>prot.versions.bash-read

No result file is cited: the row was not run in this repeat.

### <a id="prot.versions.bash-write"></a>prot.versions.bash-write

No result file is cited: the row was not run in this repeat.

### <a id="prot.versions.read"></a>prot.versions.read

No result file is cited: the row was not run in this repeat.

### <a id="prot.versions.write"></a>prot.versions.write

No result file is cited: the row was not run in this repeat.

### <a id="prot.versions.edit"></a>prot.versions.edit

No result file is cited: the row was not run in this repeat.

### <a id="prot.versions.grep"></a>prot.versions.grep

No result file is cited: the row was not run in this repeat.

### <a id="prot.versions.glob"></a>prot.versions.glob

No result file is cited: the row was not run in this repeat.

### <a id="prot.stable.bash-read"></a>prot.stable.bash-read

No result file is cited: the row was not run in this repeat.

### <a id="prot.stable.bash-write"></a>prot.stable.bash-write

No result file is cited: the row was not run in this repeat.

### <a id="prot.stable.read"></a>prot.stable.read

No result file is cited: the row was not run in this repeat.

### <a id="prot.stable.write"></a>prot.stable.write

No result file is cited: the row was not run in this repeat.

### <a id="prot.stable.edit"></a>prot.stable.edit

No result file is cited: the row was not run in this repeat.

### <a id="prot.stable.grep"></a>prot.stable.grep

No result file is cited: the row was not run in this repeat.

### <a id="prot.stable.glob"></a>prot.stable.glob

No result file is cited: the row was not run in this repeat.

### <a id="prot.stable.staged-intact"></a>prot.stable.staged-intact

No result file is cited: the row was not run in this repeat.

### <a id="prot.versions.no-stray"></a>prot.versions.no-stray

No result file is cited: the row was not run in this repeat.

### <a id="prot.bypass.read-home"></a>prot.bypass.read-home

No result file is cited: the row was not run in this repeat.

### <a id="prot.bypass.bash-write-home"></a>prot.bypass.bash-write-home

No result file is cited: the row was not run in this repeat.

### <a id="prot.bypass.hook-push"></a>prot.bypass.hook-push

No result file is cited: the row was not run in this repeat.

### <a id="prot.net.user-listed"></a>prot.net.user-listed

No result file is cited: the row was not run in this repeat.

### <a id="prot.net.unlisted"></a>prot.net.unlisted

No result file is cited: the row was not run in this repeat.

### <a id="prot.net.project-owner-host"></a>prot.net.project-owner-host

No result file is cited: the row was not run in this repeat.

### <a id="prot.net.project-own-host"></a>prot.net.project-own-host

No result file is cited: the row was not run in this repeat.

### <a id="optin.set"></a>optin.set

No result file is cited: the row was not run in this repeat.

### <a id="optin.session"></a>optin.session

No result file is cited: the row was not run in this repeat.

### <a id="optin.staged"></a>optin.staged

No result file is cited: the row was not run in this repeat.

### <a id="optin.no-wait"></a>optin.no-wait

No result file is cited: the row was not run in this repeat.

### <a id="man.setting-off"></a>man.setting-off

No result file is cited: the row was not run in this repeat.

### <a id="man.relink"></a>man.relink

No result file is cited: the row was not run in this repeat.

### <a id="man.s1.version"></a>man.s1.version

No result file is cited: the row was not run in this repeat.

### <a id="man.update.receipt"></a>man.update.receipt

`results/p7-update.txt`, section `update`:

```
== update
update outcome: staged
staged version: 0.1.0
0.1.0 is now the active version at ~/.local/share/baley-install-live/home/.local/bin/baley
new Claude Code sessions start 0.1.0; running sessions keep the version they started with
the catalog seed of 0.1.0 was recorded
kept versions: 0.0.2, 0.1.0
exit status: 0
```

### <a id="man.s1.keeps"></a>man.s1.keeps

No result file is cited: the row was not run in this repeat.

### <a id="man.s2.new"></a>man.s2.new

No result file is cited: the row was not run in this repeat.

### <a id="man.ledger"></a>man.ledger

`results/p7-update.txt`, section `events`:

```
== events
seq       type       actor  outcome  active  staged
---  --------------  -----  -------  ------  ------
  6  update.checked  owner  staged   0.1.0   0.1.0
```

### <a id="rerun.doctor-stale"></a>rerun.doctor-stale

`results/doctor-before-rerun.txt`, line 51 and line 86:

```
stub `bal-help`: ~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md is stale, since this binary no longer ships it; run baley install so the install record stops listing it
exit status: 1
```

### <a id="rerun.doctor-capture"></a>rerun.doctor-capture

`results/doctor-before-rerun.txt`, line 47:

```
stub `bal-capture`: ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md is an older stub Baley placed, not the one this version renders: expected sha256 99c85d7cf7b331280cbee3623c4c900ecb8a44637bc63f327c40a8e8ce639ba3, found sha256 1d0e290fa0d6beab59221975effdfa38ac266a1b516d0545855619573d054bf7; run baley install to bring it to this version
```

### <a id="rerun.stub-sums-before"></a>rerun.stub-sums-before

`results/p8-stub-sums-before.txt`:

```
1d0e290fa0d6beab59221975effdfa38ac266a1b516d0545855619573d054bf7  ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md
1be9fec210a65c40dcfe65da21f86bfd491932287ac3767e934947605bd4e1a9  ~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md
```

### <a id="rerun.receipt"></a>rerun.receipt

`results/p8-rerun.txt`, lines 1-7 and the last line:

```
install outcome: complete
baley 0.1.0 at ~/.local/share/baley-install-live/home/.local/bin/baley
stub `bal-capture` at ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md: replaced
registration at ~/.local/share/baley-install-live/claude-config/.claude.json: unchanged
hook at ~/.local/share/baley-install-live/claude-config/settings.json: written
settings at ~/.local/share/baley-install-live/claude-config/settings.json: written
stub `bal-help` at ~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md: removed, since this binary no longer ships it
exit status: 0
```

### <a id="rerun.record"></a>rerun.record

`results/p8-reads.txt`, section `record (step 4)`:

```
== record (step 4)
seq  binary_version  complete                                                                                                                                                                                                  stubs
---  --------------  --------  -----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  3  0.0.2                  1  [{"identity":"bal-capture","path":"~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md","sha256":"1d0e290fa0d6beab59221975effdfa38ac266a1b516d0545855619573d054bf7"},{"identity":"bal-help","path":"~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md","sha256":"1be9fec210a65c40dcfe65da21f86bfd491932287ac3767e934947605bd4e1a9"}]
  8  0.1.0                  1  [{"identity":"bal-capture","path":"~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md","sha256":"99c85d7cf7b331280cbee3623c4c900ecb8a44637bc63f327c40a8e8ce639ba3"}]
```

### <a id="rerun.capture-upgraded"></a>rerun.capture-upgraded

`results/p8-reads.txt`, section `capture digest (step 5)`:

```
== capture digest (step 5)
99c85d7cf7b331280cbee3623c4c900ecb8a44637bc63f327c40a8e8ce639ba3  ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md
stub newer bal-capture 99c85d7cf7b331280cbee3623c4c900ecb8a44637bc63f327c40a8e8ce639ba3
```

### <a id="rerun.dropped-stub-file"></a>rerun.dropped-stub-file

`results/p8-reads.txt`, section `bal-help file (step 6)`:

```
== bal-help file (step 6)
ls: cannot access '~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md': No such file or directory
total 0
drwxr-xr-x 1 john john 38 Oct 11 19:40 .
drwx------ 1 john john 76 Oct 11 19:40 ..
drwxr-xr-x 1 john john 16 Oct 11 19:40 bal-capture
drwxr-xr-x 1 john john  0 Oct 11 19:40 bal-help
```

### <a id="rerun.settings-diff"></a>rerun.settings-diff

`results/p8-reads.txt`, section `settings diff (step 6)`:

```
== settings diff (step 6)
35d34
<       "Edit(/~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md)",
55d53
<         "~/.local/share/baley-install-live/claude-config/skills/bal-help/SKILL.md",
diff exit: 1
```

### <a id="rerun.one-hook"></a>rerun.one-hook

`results/p8-reads.txt`, section `one hook (step 7)`:

```
== one hook (step 7)
1
```

### <a id="rerun.one-registration"></a>rerun.one-registration

`results/p8-reads.txt`, section `one registration (step 7)`:

```
== one registration (step 7)
["baley"]
registration same as before
```

### <a id="rerun.owner-key"></a>rerun.owner-key

`results/p8-reads.txt`, section `owner settings (step 8)`:

```
== owner settings (step 8)
30
["Read(/~/.local/share/baley-install-live/owner-private/**)"]
[{"matcher":"Bash|WebFetch","hooks":[{"type":"command","command":"true"}]}]
["example.com"]
```

### <a id="rerun.owner-deny"></a>rerun.owner-deny

`results/p8-reads.txt`, section `owner settings (step 8)`:

```
== owner settings (step 8)
30
["Read(/~/.local/share/baley-install-live/owner-private/**)"]
[{"matcher":"Bash|WebFetch","hooks":[{"type":"command","command":"true"}]}]
["example.com"]
```

### <a id="rerun.owner-hook"></a>rerun.owner-hook

`results/p8-reads.txt`, section `owner settings (step 8)`:

```
== owner settings (step 8)
30
["Read(/~/.local/share/baley-install-live/owner-private/**)"]
[{"matcher":"Bash|WebFetch","hooks":[{"type":"command","command":"true"}]}]
["example.com"]
```

### <a id="rerun.owner-host"></a>rerun.owner-host

`results/p8-reads.txt`, section `owner settings (step 8)`:

```
== owner settings (step 8)
30
["Read(/~/.local/share/baley-install-live/owner-private/**)"]
[{"matcher":"Bash|WebFetch","hooks":[{"type":"command","command":"true"}]}]
["example.com"]
```

### <a id="exit.r1.signal"></a>exit.r1.signal

No result file is cited: the row was not run in this repeat.

### <a id="exit.r1.in-flight"></a>exit.r1.in-flight

No result file is cited: the row was not run in this repeat.

### <a id="exit.r1.answered"></a>exit.r1.answered

No result file is cited: the row was not run in this repeat.

### <a id="exit.r1.server-end"></a>exit.r1.server-end

No result file is cited: the row was not run in this repeat.

### <a id="exit.r1.stderr"></a>exit.r1.stderr

No result file is cited: the row was not run in this repeat.

### <a id="exit.r2.signal"></a>exit.r2.signal

No result file is cited: the row was not run in this repeat.

### <a id="exit.r2.in-flight"></a>exit.r2.in-flight

No result file is cited: the row was not run in this repeat.

### <a id="exit.r2.answered"></a>exit.r2.answered

No result file is cited: the row was not run in this repeat.

### <a id="exit.condition"></a>exit.condition

No result file is cited: the row was not run in this repeat.

### <a id="post.verify-user"></a>post.verify-user

`results/reads.txt`, line 160:

```
exit status: 0
```

### <a id="post.views-user"></a>post.views-user

`results/reads.txt`, line 165:

```
exit status: 0
```

### <a id="post.verify-one"></a>post.verify-one

`results/reads.txt`, line 176:

```
exit status: 0
```

### <a id="post.views-one"></a>post.views-one

`results/reads.txt`, line 181:

```
exit status: 0
```

### <a id="post.verify-two"></a>post.verify-two

No result file is cited: the row was not run in this repeat.

### <a id="post.views-two"></a>post.views-two

No result file is cited: the row was not run in this repeat.

### <a id="post.doctor"></a>post.doctor

`results/reads.txt`, lines 186-271:

```
== baley doctor ==
epoch 1
integrity: ok
database 344064 bytes, log 0 bytes
project d8e043b2-1d1e-42f3-8c48-6e6c034f9818 (one): not checked against a remote from this directory
chain head 6 c2bb3bb5db0e703018a5a0d1f4fc38ed8dce3cc1612d2a1d3857aa09da79e77a
not compared with a remote anchor
unanchored sequences 1 to 6
work unanchored since 2026-10-11T23:40:06.861768402Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
unanchored age: since 2026-10-11T23:40:06.861768402Z
view set 8, binary 8
view capture: live 1, binary 1
view checkout: live 1, binary 1
view claim_scope: live 1, binary 1
view guard: live 1, binary 1
view guard_policy: live 1, binary 1
view install: live 1, binary 1
view model_catalog: live 1, binary 1
view policy: live 1, binary 1
view request: live 2, binary 2
views checked at sequence 6
no differences
claims: 0 active, 0 interrupted, 0 awaiting owner
project user (per-user records): not checked against a remote from this directory
chain head 9 027d61f745efd4dc60fcb4a74cf13e03ce4eb3428f75a5969fa905c4dfc130ac
not compared with a remote anchor
unanchored sequences 1 to 9
work unanchored since 2026-10-11T23:40:00.625023881Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
unanchored age: since 2026-10-11T23:40:00.625023881Z
view set 8, binary 8
view capture: live 1, binary 1
view checkout: live 1, binary 1
view claim_scope: live 1, binary 1
view guard: live 1, binary 1
view guard_policy: live 1, binary 1
view install: live 1, binary 1
view model_catalog: live 1, binary 1
view policy: live 1, binary 1
view request: live 2, binary 2
views checked at sequence 9
no differences
claims: 0 active, 0 interrupted, 0 awaiting owner
Claude Code host checks
stub `bal-capture`: ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md matches the stub Baley renders, sha256 99c85d7cf7b331280cbee3623c4c900ecb8a44637bc63f327c40a8e8ce639ba3
registration: ~/.local/share/baley-install-live/claude-config/.claude.json holds the entry Baley renders for this binary
hook: ~/.local/share/baley-install-live/claude-config/settings.json holds every Baley entry that install writes and the install record holds
settings: ~/.local/share/baley-install-live/claude-config/settings.json holds every Baley entry that install writes and the install record holds
stable path: ~/.local/share/baley-install-live/home/.local/bin/baley links to staged version 0.1.0, the version new Claude Code sessions start
installed locations: ~/.local/share/baley-install-live/home/.local/bin, ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions, ~/.local/share/baley-install-live/claude-config/skills/bal-capture, ~/.local/share/baley-install-live/claude-config are writable
sandbox: bwrap found at /usr/bin/bwrap
sandbox: socat found at /usr/bin/socat
coverage judged over the settings and hook in ~/.local/share/baley-install-live/claude-config/settings.json
tool Bash, read access: configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json
tool Bash, write access: configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json
tool Monitor, read access: configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json
tool Monitor, write access: configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json
tool PowerShell, read access: configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json
tool PowerShell, write access: configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json
tool Read, read access: configured as covered by the Read deny rules in ~/.local/share/baley-install-live/claude-config/settings.json and the guard hook in ~/.local/share/baley-install-live/claude-config/settings.json
tool Grep, read access: configured as covered by the Read deny rules in ~/.local/share/baley-install-live/claude-config/settings.json and the guard hook in ~/.local/share/baley-install-live/claude-config/settings.json (best-effort)
tool Glob, read access: configured as covered by the Read deny rules in ~/.local/share/baley-install-live/claude-config/settings.json and the guard hook in ~/.local/share/baley-install-live/claude-config/settings.json (best-effort)
tool Write, write access: configured as covered by the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json and the guard hook in ~/.local/share/baley-install-live/claude-config/settings.json
tool Edit, write access: configured as covered by the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json and the guard hook in ~/.local/share/baley-install-live/claude-config/settings.json
tool NotebookEdit, write access: configured as covered by the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json and the guard hook in ~/.local/share/baley-install-live/claude-config/settings.json
write-only file ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md: write access configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json and the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json
write-only file ~/.local/share/baley-install-live/claude-config/.claude.json: write access configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json and the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json
write-only file ~/.local/share/baley-install-live/claude-config/settings.json: write access configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json and the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json
write-only file ~/.local/share/baley-install-live/home/.local/bin/baley: write access configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json and the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json
These verdicts describe configuration in ~/.local/share/baley-install-live/claude-config/settings.json and are not proof that Claude Code enforces it.
Other Claude Code settings files (managed, command line, local, project and user) and the sandbox.enabledPlatforms policy setting can change what applies: lists combine across files and a single value comes from the highest file.
guard hook: a PreToolUse item in ~/.local/share/baley-install-live/claude-config/settings.json runs the guard for all nine tools in the matcher
provider hosts: sandbox.network.allowedDomains in ~/.local/share/baley-install-live/claude-config/settings.json allows no review provider's API host
provider acknowledgement `openai`: none is recorded
provider acknowledgement `deepseek`: none is recorded
updates.auto: false, the default, since the global settings do not set it
server context: no server call is recorded in the ledger yet
compatibility: the ledger is at epoch 1, the epoch of this binary
write fence: the server's startup quick_check is not observed from the command line, and the integrity_check rows above stand in for it
home: ~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley opened with its ownership, mode and link checks passed
config folder: ~/.local/share/baley-install-live/home/.config/crenshawdev/baley
guard records: the views of the user project are current for this binary
exit status: 0
```

### <a id="post.owner-settings"></a>post.owner-settings

`results/reads.txt`, lines 303-307:

```
== settings: the seeded owner entries against results/owner-settings.json ==
owner key cleanupPeriodDays: kept
owner permissions.deny rule: kept
owner PreToolUse item: kept
owner sandbox.network.allowedDomains host: kept
```

### <a id="post.real-installation"></a>post.real-installation

`results/reads.txt`, lines 318-321:

```
== the owner's real installation against pins.txt (a difference is shown, left: pins.txt, right: now) ==
~/.local/bin/baley, ~/.claude/settings.json, the mcpServers.baley entry and the claude launcher: no difference
~/.local/share/crenshawdev/baley: no difference
~/.config/crenshawdev/baley: no difference
```

### <a id="post.real-claude"></a>post.real-claude

`results/reads.txt`, lines 323-325:

```
== where the claude command and the launcher resolve now ==
the claude command resolves outside both disposable roots (~/.local/share/claude/versions/2.1.296)
the launcher resolves outside both disposable roots (~/.local/share/claude/versions/2.1.296)
```

## Post-run reads


== pins ==
date: 2026-10-11T23:39:38Z
platform: Linux 7.2.9-zen1-1-zen x86_64
bwrap: /usr/bin/bwrap
socat: /usr/bin/socat
sqlite3: /usr/bin/sqlite3
jq: /usr/bin/jq
curl: /usr/bin/curl
sha256sum: /usr/bin/sha256sum
shasum: /usr/bin/core_perl/shasum
git: /usr/bin/git
claude: ~/.local/bin/claude
checkout-commit: b1ec13d57b53477476ccca39e29cc97f21a8143a
checkout-status: clean
begin artifacts ~/.local/share/baley-install-artifacts/artifacts.txt
date: 2026-10-11T23:33:29Z
platform: linux-x86_64
commit to qualify: b1ec13d57b53477476ccca39e29cc97f21a8143a
older source commit: beba9141de2af185b3676bfd2265d77b829ef14d
marker sentence added to the bal-capture stub of newer: This stub was changed for the install rerun check.
bal-help is left out of newer's manifest by dropping it from front_doors in crates/baley/src/host_artifacts/stubs.rs
artifact older: version 0.0.1 sha256 8b22d5f3f6809813e4bf7dda10d8f86f6def106ec02603d7bf160094604b4135 commit beba9141de2af185b3676bfd2265d77b829ef14d edits Cargo.lock crates/baley/Cargo.toml 
artifact base: version 0.0.2 sha256 88a6c856145735eb3b304a5573e910ffb279f4ced2f91070f86dc932f700aba6 commit b1ec13d57b53477476ccca39e29cc97f21a8143a edits Cargo.lock crates/baley/Cargo.toml 
artifact newer: version 0.1.0 sha256 5498313d585575c30786b20023c92baa5b9c06a0bc843792683240f3b5830f1a commit b1ec13d57b53477476ccca39e29cc97f21a8143a edits crates/baley/src/help/table.rs crates/baley/src/host_artifacts/stubs.rs 
begin manifest base
[
  {
    "host": "claude-code",
    "identity": "bal-capture",
    "digest": "1d0e290fa0d6beab59221975effdfa38ac266a1b516d0545855619573d054bf7"
  },
  {
    "host": "claude-code",
    "identity": "bal-help",
    "digest": "1be9fec210a65c40dcfe65da21f86bfd491932287ac3767e934947605bd4e1a9"
  }
]
end manifest base
stub base bal-capture 1d0e290fa0d6beab59221975effdfa38ac266a1b516d0545855619573d054bf7
stub base bal-help 1be9fec210a65c40dcfe65da21f86bfd491932287ac3767e934947605bd4e1a9
begin manifest newer
[
  {
    "host": "claude-code",
    "identity": "bal-capture",
    "digest": "99c85d7cf7b331280cbee3623c4c900ecb8a44637bc63f327c40a8e8ce639ba3"
  }
]
end manifest newer
stub newer bal-capture 99c85d7cf7b331280cbee3623c4c900ecb8a44637bc63f327c40a8e8ce639ba3
begin edits older
diff --git a/Cargo.lock b/Cargo.lock
index e47638f9..45e50c30 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -101,7 +101,7 @@ checksum = "f2032f911046de80f0a198e0901378627c33f59ea0ac00e363d481118bd70a53"
 
 [[package]]
 name = "baley"
-version = "0.1.0"
+version = "0.0.1"
 dependencies = [
  "aho-corasick",
  "baley-core",
diff --git a/crates/baley/Cargo.toml b/crates/baley/Cargo.toml
index 407b9509..41a1313a 100644
--- a/crates/baley/Cargo.toml
+++ b/crates/baley/Cargo.toml
@@ -1,6 +1,6 @@
 [package]
 name = "baley"
-version = "0.1.0"
+version = "0.0.1"
 edition = "2024"
 license = "MIT"
 description = "Baley, the owner's control plane for AI-assisted engineering: the binary that serves MCP, guards tool calls and keeps the evidence ledger."
end edits older
begin edits base
diff --git a/Cargo.lock b/Cargo.lock
index e47638f9..bfb51fa6 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -101,7 +101,7 @@ checksum = "f2032f911046de80f0a198e0901378627c33f59ea0ac00e363d481118bd70a53"
 
 [[package]]
 name = "baley"
-version = "0.1.0"
+version = "0.0.2"
 dependencies = [
  "aho-corasick",
  "baley-core",
diff --git a/crates/baley/Cargo.toml b/crates/baley/Cargo.toml
index 407b9509..e6c7935a 100644
--- a/crates/baley/Cargo.toml
+++ b/crates/baley/Cargo.toml
@@ -1,6 +1,6 @@
 [package]
 name = "baley"
-version = "0.1.0"
+version = "0.0.2"
 edition = "2024"
 license = "MIT"
 description = "Baley, the owner's control plane for AI-assisted engineering: the binary that serves MCP, guards tool calls and keeps the evidence ledger."
end edits base
begin edits newer
diff --git a/crates/baley/src/help/table.rs b/crates/baley/src/help/table.rs
index e57dcd9d..b91ca6e2 100644
--- a/crates/baley/src/help/table.rs
+++ b/crates/baley/src/help/table.rs
@@ -103,7 +103,7 @@ pub const COMMANDS: &[Command] = &[
     Command {
         name: "bal-capture",
         cluster: "Support",
-        description: "Record a note, or a story candidate for the backlog, in the ledger.",
+        description: "Record a note, or a story candidate for the backlog, in the ledger. This stub was changed for the install rerun check.",
     },
     Command {
         name: "bal-help",
diff --git a/crates/baley/src/host_artifacts/stubs.rs b/crates/baley/src/host_artifacts/stubs.rs
index 1bb15ece..eb64b018 100644
--- a/crates/baley/src/host_artifacts/stubs.rs
+++ b/crates/baley/src/host_artifacts/stubs.rs
@@ -24,6 +24,7 @@ pub fn front_doors() -> Vec<(&'static str, &'static str)> {
     COMMANDS
         .iter()
         .filter(|command| matches!(instruction::lookup(command.name), Lookup::Served { .. }))
+        .filter(|command| command.name != "bal-help")
         .map(|command| (command.name, command.description))
         .collect()
 }
end edits newer
end artifacts
real-baley: absent
real-baley-sha256: absent
real-claude-settings-sha256: 3a5b8d123dfca1da11d2c7d45206a19f57d0dfd1679ead53dcf81d88bc37d45d
real-claude-registration-sha256: 70056b5ee93a7f08f583971157cf1ea50b8f27cbb89371e308a2a7b984b43590
claude-launcher: link to ~/.local/share/claude/versions/2.1.296
begin real-home ~/.local/share/crenshawdev/baley
~/.local/share/crenshawdev/baley/baley.db f 286720 2026-10-02 12:28:05.0100125140
~/.local/share/crenshawdev/baley/baley.db.maintenance f 0 2026-10-02 12:25:03.5899264730
~/.local/share/crenshawdev/baley/baley.db.writer f 0 2026-10-02 12:25:03.5899264730
~/.local/share/crenshawdev/baley d 86 2026-10-10 09:43:49.4310351150
end real-home
begin real-config ~/.config/crenshawdev/baley
~/.config/crenshawdev/baley d 16 2026-09-30 16:08:55.8750781170
~/.config/crenshawdev/baley/keys.env f 183 2026-09-30 16:08:55.8767744790
end real-config
project ids: one=d8e043b2-1d1e-42f3-8c48-6e6c034f9818 two=none

== baley verify --local-only user ==
local only
checked at 2026-10-11T23:40:39.930812578Z
chain head 9 027d61f745efd4dc60fcb4a74cf13e03ce4eb3428f75a5969fa905c4dfc130ac
not compared with a remote anchor
unanchored sequences 1 to 9
work unanchored since 2026-10-11T23:40:00.625023881Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
exit status: 0

== baley verify --views user ==
views checked at sequence 9
no differences
exit status: 0

== baley verify --local-only d8e043b2-1d1e-42f3-8c48-6e6c034f9818 ==
local only
checked at 2026-10-11T23:40:39.957926806Z
chain head 6 c2bb3bb5db0e703018a5a0d1f4fc38ed8dce3cc1612d2a1d3857aa09da79e77a
not compared with a remote anchor
unanchored sequences 1 to 6
work unanchored since 2026-10-11T23:40:06.861768402Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
exit status: 0

== baley verify --views d8e043b2-1d1e-42f3-8c48-6e6c034f9818 ==
views checked at sequence 6
no differences
exit status: 0

== verify for a project with no baley.toml ==
skipped: that project was not initialised

== baley doctor ==
epoch 1
integrity: ok
database 344064 bytes, log 0 bytes
project d8e043b2-1d1e-42f3-8c48-6e6c034f9818 (one): not checked against a remote from this directory
chain head 6 c2bb3bb5db0e703018a5a0d1f4fc38ed8dce3cc1612d2a1d3857aa09da79e77a
not compared with a remote anchor
unanchored sequences 1 to 6
work unanchored since 2026-10-11T23:40:06.861768402Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
unanchored age: since 2026-10-11T23:40:06.861768402Z
view set 8, binary 8
view capture: live 1, binary 1
view checkout: live 1, binary 1
view claim_scope: live 1, binary 1
view guard: live 1, binary 1
view guard_policy: live 1, binary 1
view install: live 1, binary 1
view model_catalog: live 1, binary 1
view policy: live 1, binary 1
view request: live 2, binary 2
views checked at sequence 6
no differences
claims: 0 active, 0 interrupted, 0 awaiting owner
project user (per-user records): not checked against a remote from this directory
chain head 9 027d61f745efd4dc60fcb4a74cf13e03ce4eb3428f75a5969fa905c4dfc130ac
not compared with a remote anchor
unanchored sequences 1 to 9
work unanchored since 2026-10-11T23:40:00.625023881Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
unanchored age: since 2026-10-11T23:40:00.625023881Z
view set 8, binary 8
view capture: live 1, binary 1
view checkout: live 1, binary 1
view claim_scope: live 1, binary 1
view guard: live 1, binary 1
view guard_policy: live 1, binary 1
view install: live 1, binary 1
view model_catalog: live 1, binary 1
view policy: live 1, binary 1
view request: live 2, binary 2
views checked at sequence 9
no differences
claims: 0 active, 0 interrupted, 0 awaiting owner
Claude Code host checks
stub `bal-capture`: ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md matches the stub Baley renders, sha256 99c85d7cf7b331280cbee3623c4c900ecb8a44637bc63f327c40a8e8ce639ba3
registration: ~/.local/share/baley-install-live/claude-config/.claude.json holds the entry Baley renders for this binary
hook: ~/.local/share/baley-install-live/claude-config/settings.json holds every Baley entry that install writes and the install record holds
settings: ~/.local/share/baley-install-live/claude-config/settings.json holds every Baley entry that install writes and the install record holds
stable path: ~/.local/share/baley-install-live/home/.local/bin/baley links to staged version 0.1.0, the version new Claude Code sessions start
installed locations: ~/.local/share/baley-install-live/home/.local/bin, ~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions, ~/.local/share/baley-install-live/claude-config/skills/bal-capture, ~/.local/share/baley-install-live/claude-config are writable
sandbox: bwrap found at /usr/bin/bwrap
sandbox: socat found at /usr/bin/socat
coverage judged over the settings and hook in ~/.local/share/baley-install-live/claude-config/settings.json
tool Bash, read access: configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json
tool Bash, write access: configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json
tool Monitor, read access: configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json
tool Monitor, write access: configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json
tool PowerShell, read access: configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json
tool PowerShell, write access: configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json
tool Read, read access: configured as covered by the Read deny rules in ~/.local/share/baley-install-live/claude-config/settings.json and the guard hook in ~/.local/share/baley-install-live/claude-config/settings.json
tool Grep, read access: configured as covered by the Read deny rules in ~/.local/share/baley-install-live/claude-config/settings.json and the guard hook in ~/.local/share/baley-install-live/claude-config/settings.json (best-effort)
tool Glob, read access: configured as covered by the Read deny rules in ~/.local/share/baley-install-live/claude-config/settings.json and the guard hook in ~/.local/share/baley-install-live/claude-config/settings.json (best-effort)
tool Write, write access: configured as covered by the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json and the guard hook in ~/.local/share/baley-install-live/claude-config/settings.json
tool Edit, write access: configured as covered by the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json and the guard hook in ~/.local/share/baley-install-live/claude-config/settings.json
tool NotebookEdit, write access: configured as covered by the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json and the guard hook in ~/.local/share/baley-install-live/claude-config/settings.json
write-only file ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md: write access configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json and the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json
write-only file ~/.local/share/baley-install-live/claude-config/.claude.json: write access configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json and the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json
write-only file ~/.local/share/baley-install-live/claude-config/settings.json: write access configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json and the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json
write-only file ~/.local/share/baley-install-live/home/.local/bin/baley: write access configured as covered by the sandbox in ~/.local/share/baley-install-live/claude-config/settings.json and the Edit deny rules in ~/.local/share/baley-install-live/claude-config/settings.json
These verdicts describe configuration in ~/.local/share/baley-install-live/claude-config/settings.json and are not proof that Claude Code enforces it.
Other Claude Code settings files (managed, command line, local, project and user) and the sandbox.enabledPlatforms policy setting can change what applies: lists combine across files and a single value comes from the highest file.
guard hook: a PreToolUse item in ~/.local/share/baley-install-live/claude-config/settings.json runs the guard for all nine tools in the matcher
provider hosts: sandbox.network.allowedDomains in ~/.local/share/baley-install-live/claude-config/settings.json allows no review provider's API host
provider acknowledgement `openai`: none is recorded
provider acknowledgement `deepseek`: none is recorded
updates.auto: false, the default, since the global settings do not set it
server context: no server call is recorded in the ledger yet
compatibility: the ledger is at epoch 1, the epoch of this binary
write fence: the server's startup quick_check is not observed from the command line, and the integrity_check rows above stand in for it
home: ~/.local/share/baley-install-live/home/.local/share/crenshawdev/baley opened with its ownership, mode and link checks passed
config folder: ~/.local/share/baley-install-live/home/.config/crenshawdev/baley
guard records: the views of the user project are current for this binary
exit status: 0

== install.recorded in sequence order ==
seq           recorded_at            binary_version  complete  held_back  hook_is_null
---  ------------------------------  --------------  --------  ---------  ------------
  3  2026-10-11T23:40:00.631295554Z  0.0.2                  1                        0
  8  2026-10-11T23:40:25.192729893Z  0.1.0                  1                        0

== update.checked and update.failed ==
seq       type       actor           recorded_at               day      outcome_or_code  active_version  staged_version
---  --------------  -----  ------------------------------  ----------  ---------------  --------------  --------------
  6  update.checked  owner  2026-10-11T23:40:12.884494335Z  2026-10-11  staged           0.1.0           0.1.0

== update.check claims ==
seq  actor           recorded_at                         request_id
---  -----  ------------------------------  ------------------------------------
  5  owner  2026-10-11T23:40:12.168885301Z  7716e00a-74ee-4abc-8ac4-22d75d40980b

== captures per project: recorded_at, request id and the caller's baley_session ==

== guard answers in user by decision ==

== the stable link's stored target ==
~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/0.1.0/baley

== versions folder: each child with its baley's modification time in UTC (when its download finished) ==
~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/0.0.2 2026-10-11T23:40:00.3657862350Z 19168344 bytes
~/.local/share/baley-install-live/home/.local/lib/crenshawdev/baley/versions/0.1.0 2026-10-11T23:40:12.8662988840Z 19168272 bytes

== settings: Baley PreToolUse items that run guard at the disposable stable path ==
1

== settings: the seeded owner entries against results/owner-settings.json ==
owner key cleanupPeriodDays: kept
owner permissions.deny rule: kept
owner PreToolUse item: kept
owner sandbox.network.allowedDomains host: kept

== settings: sandbox.network.allowedDomains ==
["example.com"]

== registration: the keys of mcpServers in ~/.local/share/baley-install-live/claude-config/.claude.json ==
["baley"]

== stubs under ~/.local/share/baley-install-live/claude-config/skills with their SHA-256 ==
99c85d7cf7b331280cbee3623c4c900ecb8a44637bc63f327c40a8e8ce639ba3  ~/.local/share/baley-install-live/claude-config/skills/bal-capture/SKILL.md

== the owner's real installation against pins.txt (a difference is shown, left: pins.txt, right: now) ==
~/.local/bin/baley, ~/.claude/settings.json, the mcpServers.baley entry and the claude launcher: no difference
~/.local/share/crenshawdev/baley: no difference
~/.config/crenshawdev/baley: no difference

== where the claude command and the launcher resolve now ==
the claude command resolves outside both disposable roots (~/.local/share/claude/versions/2.1.296)
the launcher resolves outside both disposable roots (~/.local/share/claude/versions/2.1.296)
