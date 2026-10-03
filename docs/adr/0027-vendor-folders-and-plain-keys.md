# 0027: Keep Baley's files in its own crenshawdev folders, with provider keys in a plain keys.env

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-28 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md), [0002: System design](../design/0002-system-design.md), [0003: Configuration and routing](../design/0003-configuration-and-routing.md) |
| Supersedes | [0015](0015-settings-in-toml.md), in part: the global file's location and name; [0016](0016-key-store.md); [0003](0003-per-user-database.md), in part: the data location; [0002](0002-sqlite.md), in part: detecting and refusing a network filesystem; [0013](0013-host-session-calls-outside-models.md), in part: where keys are kept and the name `baley exec --key` takes; [0020](0020-sandbox-is-a-write-barrier.md), in part: the statements about the master key and encrypted keys |
| Superseded by | [0032](0032-gemini-is-not-a-provider.md), in part: the key-name table names `OPENAI_API_KEY` and `DEEPSEEK_API_KEY` only, since Gemini is no provider detection knows; [0033](0033-host-security-bar.md), in part: the Codex consequence |

## Context and problem

ADR 0015 put the global settings file at `~/.config/baley/baley.toml` on Linux and `~/Library/Application Support/baley/baley.toml` on macOS. ADR 0003 put the ledger at `$XDG_DATA_HOME/baley` or `~/.local/share/baley` on Linux and `~/Library/Application Support/baley` on macOS. Baley is one of the applications published under the crenshawdev name, and each of them needs its own place for configuration and data.

ADR 0016 stored provider API keys encrypted in a `secret` table of the ledger, with the master key in macOS Keychain or the Linux Secret Service, and in a mode-0600 file where neither was reachable. The ledger, the fallback file and the owner's other files are all protected by the same operating-system user. The Linux Secret Service answers any process running as that user, and on Codex an agent can read every file the user can read (ADR 0020). Other tools that call the same providers take keys from environment variables or plain files under the same protection. Encryption would add a master key, a secret store client on each platform, a fallback path and a secure random source, and would protect only a database copied without its home.

ADR 0002 required the store to detect a network filesystem and refuse it. SQLite's write-ahead log needs every process using the database on one host computer and does not work over a network filesystem ([sqlite.org/wal.html](https://www.sqlite.org/wal.html)). The filesystem type does not answer the question reliably: on Linux, `statfs` returns one magic number for every FUSE filesystem, local or remote, and `libc` 0.2.189 defines no constant for CIFS or SMB2.

Build 2, which adds settings and keys, also needs a TOML reader and writer, the platform folders, ownership, mode and symbolic-link checks, a safe way to replace a settings file, redaction of a key in a command's output, a random project id and an HTTP client for provider model lists. Each option was measured by what it adds to `Cargo.lock` and to the packages compiled, and by whether `cargo deny --locked check` passes.

## Decision drivers

- Each platform's own configuration and data locations, grouped under one vendor folder.
- No application reads or writes another application's files.
- Keys are protected by the same boundary as everything else the owner's user holds, and Baley adds no mechanism that looks stronger than it is.
- The owner holds the keys and edits them; Baley only reads them.
- A check Baley makes on every open is one it can make correctly on both platforms.
- Few new packages; a library already built is preferred to a new one; every pick passes `cargo deny`.
- The decisions stay testable with plain values.

## Considered options

Folders:

1. One folder per application at the platform root (`~/.config/baley`), as ADR 0015 and ADR 0003 had it
2. A vendor folder holding one folder per application (`~/.config/crenshawdev/baley`)
3. A vendor folder whose files the applications share

Provider keys:

4. Encrypted in the ledger with the master key in the OS secret store (ADR 0016)
5. Read from environment variables
6. Plain `NAME=value` lines in one file, `keys.env`, in Baley's config folder, edited by the owner
7. In the project file `baley.toml`

Network filesystems:

8. Detect a network filesystem when the store opens and refuse
9. State that network shares are not supported, with no check

Libraries: one choice per need, listed in [Libraries](#libraries).

## Decision

Chosen options: **2, 6 and 9**, and the libraries below.

**Folders.** Baley's config folder is `$XDG_CONFIG_HOME/crenshawdev/baley` on Linux (`~/.config/crenshawdev/baley` by default) and `~/Library/Application Support/crenshawdev/baley` on macOS. Its data folder, which holds the ledger, is `$XDG_DATA_HOME/crenshawdev/baley` on Linux (`~/.local/share/crenshawdev/baley` by default) and the same `~/Library/Application Support/crenshawdev/baley` on macOS. An empty or relative `XDG_CONFIG_HOME` or `XDG_DATA_HOME` is treated as unset. The global settings file is `config.toml` in the config folder; the project file stays `baley.toml` at the repository root (ADR 0004). Every crenshawdev application has its own folder under the vendor folder, and nothing is shared at the vendor level, keys included. When `BALEY_HOME` is set, `config.toml`, `keys.env` and the database all live in it, and the crenshawdev folders are not used. The host sandbox and the guard, built with the hosts in Build 3, keep agents from writing anywhere in the config folder, `config.toml` and `keys.env` included, and on Claude Code from reading it, as they do the home. Codex cannot deny reads (ADR 0020).

**Keys.** Provider API keys are plain `NAME=value` lines in one file, `keys.env`, in Baley's config folder beside `config.toml`, for example `OPENAI_API_KEY=...`. A line may start with `export `, a value may be in single or double quotes, and a line starting with `#` is a comment; a name that appears twice is refused with `keys-file-invalid`, naming the line. The owner edits the file by hand. Baley only reads it and has no command that sets, removes or lists keys. Baley refuses to read the file with `keys-file-exposed` when its group or others can read it, naming the file and the `chmod 600` fix, or when another user owns it, naming the `chown` fix. A symbolic link to the file is followed. The check does not check the folder; where the folder is Baley's home (on macOS, or when `BALEY_HOME` is set), the home's own open checks apply. Keys come only from that file, never from environment variables. There is no encryption, no master key and no OS secret store. `baley exec --key <NAME> -- <command>` takes the name exactly as it is written in `keys.env`, sets that same name in the command's environment, and replaces the key in the command's output with `[baley:<NAME>]`, such as `[baley:OPENAI_API_KEY]`. Detection finds a provider's key through a small compiled table: `OPENAI_API_KEY`, `GEMINI_API_KEY`, `DEEPSEEK_API_KEY`. The ledger may record that a key was used and how, for example a review by OpenAI through its API with `OPENAI_API_KEY`, but never the key's value.

**Network shares.** Baley makes no filesystem check. A data folder on a network share is not supported, because SQLite's write-ahead log does not work over a network filesystem.

**Libraries.**

- TOML: the `toml` crate, 1.1.6. When Baley writes a TOML file with it, the file's comments and key order are not kept.
- Folders: std, from `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `HOME`, joined with `crenshawdev/baley`. No crate.
- Ownership, mode and symbolic-link checks: std, with `libc` (already a direct dependency) for the current user id, `geteuid`. No crate.
- Replacing a settings file: std, writing a temporary file in the same folder and renaming it over the target.
- Redacting a key in `baley exec` output: `aho-corasick`, as a direct dependency. It is already built through `regex`.
- The project id: `uuid` with `v4`, as a direct dependency. It is already built through `rmcp`.
- Provider model-list calls: `reqwest` 0.12, already a direct dependency, with rustls and the ring provider. [ADR 0028](0028-one-http-stack.md) records the HTTP stack it belongs to.

## Consequences

### Positive

- One place per platform for everything Baley keeps, and room for other crenshawdev applications beside it without collisions.
- `BALEY_HOME` gives tests and development builds one folder holding everything.
- Keys are kept the way the owner already keeps keys for other tools. There is no secret store client, master key, fallback file or nonce handling to build and test on two platforms.
- No filesystem classification to get wrong on every open.
- The only new packages are the six of the TOML stack (`toml`, `toml_parser`, `toml_datetime`, `toml_writer`, `serde_spanned`, `winnow`), all pure Rust.

### Negative

- Keys are protected by the file's mode and the operating-system user only. An agent under a host whose sandbox allows reads (Codex, ADR 0020) can read `keys.env`, as it can any file the owner's user can read.
- When `config set` or the settings interview rewrites a settings file, comments and key order the owner wrote there are lost.
- A data folder on a network share is not refused, and it is not supported.
- An owner who keeps keys in environment variables copies them into `keys.env` before Baley can use them.
- On Linux, configuration and data are two folders; on macOS they are one.

### Follow-up

- Build 2 ([#23](https://github.com/crenshawdev/baley/issues/23)) builds the folders, the `keys.env` reader with its ownership, mode and duplicate-name checks, `baley exec --key`, detection with `reqwest`, and the open checks of EVD-R22.
- Build 3 ([#24](https://github.com/crenshawdev/baley/issues/24)) extends the host sandbox and the guard to the whole config folder: writes denied on both hosts, reads denied on Claude Code.

## Options in detail

### Folders

**One folder per application at the platform root.** What ADR 0015 and ADR 0003 described. Nothing groups the applications of one publisher, and each takes a name at the top of `~/.config` and `~/.local/share`.

**A vendor folder holding one folder per application (chosen).** The platform's own roots, one level deeper. The folder crates do not produce this layout on both platforms (see [Libraries](#libraries)), so Baley joins the paths itself.

**A vendor folder whose files the applications share.** Applications could share keys or settings, but each could then read and change what another relies on. Not chosen: each application keeps its own folder, even where two use the same key.

### Provider keys

**Encrypted in the ledger with the master key in the OS secret store.** ADR 0016. Needs encryption, a secure random source, a Keychain client on macOS, a Secret Service client over D-Bus on Linux, a fallback file, and tests for each path. On Linux the Secret Service answers any process running as the owner, the same boundary as a file mode, and the fallback file sits beside the database it protects.

**Environment variables.** What many tools read. A variable is inherited by every process the shell starts, agents included, and Baley could not tell where a key came from. Not chosen: keys come only from `keys.env`.

**Plain lines in `keys.env` (chosen).** A file format the owner already writes for other tools, in Baley's own config folder. The file's mode is the protection, and Baley refuses the file when group or others can read it. Adds no dependency.

**In the project file.** The project file is committed, so a key there would reach the repository. Not chosen.

### Network filesystems

**Detect and refuse.** macOS reports a local filesystem through the `MNT_LOCAL` flag of `statfs`. Linux reports only a magic number: FUSE covers local and remote filesystems alike, and `libc` has no constant for CIFS or SMB2. Reading `/proc/self/mountinfo` gives type names, but which of them count as remote is a list Baley would have to keep. A wrong answer refuses a local disk or passes a remote one, on every open, including each guard call.

**State that network shares are not supported (chosen).** The limit is SQLite's, stated where the owner reads about the data folder. No code and nothing to misclassify.

### Libraries

Lock is the number of packages added to `Cargo.lock`; built is the number added to what compiles on some target. Both are measured against the lock at `325b2cc7`. Deny is `cargo deny --locked check`.

| Need | Option | Cost |
|---|---|---|
| TOML | `toml` 1.1.6 (chosen) | Lock +6, built +6, pure Rust, passes deny. Writing drops comments and key order |
| | `toml_edit` 0.25.15 | Lock +6 (the same parser stack). Keeps comments, spacing and order when editing |
| | `basic-toml` 0.1.10 | Lock +1. TOML 0.5 without dates and times; one release in two years |
| | `toml-span` 0.7.1 | Lock +1. Reads with positions; cannot write |
| | Hand-written | Lock +0. A parser, error positions and a writer for Baley to maintain; files other tools accept could be refused |
| Folders | std (chosen) | Lock +0. Baley joins the paths as a pure function of supplied values |
| | `dirs` 7.0.0, `directories` 6.0.0 | Lock +5 each. Deny fails: their `option-ext` dependency is MPL-2.0 |
| | `etcetera` 0.11.0 | Lock +1. Its macOS config folder is `~/Library/Preferences`, not Application Support |
| | `xdg` 3.0.0 | Lock +1. Linux paths only; macOS still needs Baley's own code |
| Owner, mode, links | std with `libc` (chosen) | Lock +0. One `unsafe` call for `geteuid` |
| | `rustix` 1.1.4 | Lock +0, built +2 |
| | `nix` 0.31.3 | Lock +1, built +2 |
| File replacement | std (chosen) | Lock +0 |
| | `tempfile` 3.27.0 as a normal dependency | Lock +0, built +4 |
| Redaction | `aho-corasick` 1.1.5 (chosen) | Lock +0, built +0. Replaces across a stream with a bounded buffer |
| | `memchr` 2.8.3 | Lock +0. One pattern; Baley writes the streaming |
| | Hand-written | Lock +0. A search that keeps the tail of each chunk |
| Project id | `uuid` 1.26 with `v4` (chosen) | No new package |
| | Hand-written | Lock +0. Sixteen random bytes from `libc`, the version and variant bits, and the formatting |
| Model lists | `reqwest` 0.12 (chosen) | Lock +0. Async, on the tokio runtime Baley already builds |
| | `reqwest` 0.13.5 | Built +33. Its default TLS is aws-lc-rs with the OS trust store |
| | `ureq` 3.4.2 | Built +3 without gzip, +8 with it. A second, synchronous HTTP stack |
| | `minreq` 3.0.0 | Lock +6, including aws-lc-rs |
| | `attohttpc` 0.31.0 | Deny fails: MPL-2.0 |
| | `curl` through the process seam | Lock +0. Not present on every Linux install |
