# 0016: Store provider API keys encrypted in the ledger with the master key in the OS secret store

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-26 |
| Deciders | John Crenshaw |
| Design document | [0003: Configuration and routing](../design/0003-configuration-and-routing.md) |
| Supersedes |  |
| Superseded by | |

## Context and problem

Baley needs provider API keys for outside calls (`baley exec --key`, ADR 0013) and for model detection. Today's engine reads them from environment variables and a plain `providers.env` file. The owner's requirements: Baley stores the keys itself, out of reasonable reach, never in a plain file in a folder, never in environment variables, managed only through Baley's command line (SYS-R12). A passphrase was rejected.

Encryption moves the question to where the encryption key lives: a passphrase, the operating system's secret store, or a file beside the database. A file beside the database protects only a database copied without its home.

## Decision drivers

- Keys are Baley's to hold and are never read from a file or the environment.
- The strongest protection the machine offers is used, and the owner is told when it is not the strongest.
- Keys never enter the record, backups or exports.
- No passphrase.

## Considered options

1. Plain in the database, protected by file modes
2. Encrypted in the database with a master key in a file beside it
3. Encrypted in the database with the master key in the OS secret store, falling back to a file only where no secret store exists

## Decision

Chosen option: **3**. API keys are stored encrypted in a `secret` table of the per-user database. The master key lives in macOS Keychain or the Linux Secret Service when one is reachable; where none is, it is a file in the Baley home with mode 0600, and the owner is told the residual risk. Baley picks the most secure mechanism available and records which is in use. `baley key set` reads a key from the terminal, `baley key remove` and `baley key list` (names only) manage them. The ledger records that a key exists and when it was set, never the key; the secret table is excluded from backups, exports and views. Baley reads a key for `baley exec --key` injection and model detection only.

## Consequences

### Positive

- Keys are out of the environment, out of plain files, and, on macOS, protected per application.
- The mechanism in use is a recorded fact the owner can check with `baley doctor`.
- Backups and exports stay shareable.

### Negative

- On Linux the protection is per user, the same as file modes; the design says so.
- Two code paths for the master key, one of them the fallback, both to test.

### Follow-up

- Build the secret table with the key commands; probe both secret stores on the release platforms.

## Options in detail

### Plain in the database

Meets the letter of no plain file and no environment variable; a copied database carries the keys. Fails the second driver.

### Master key in a file beside the database

Adds encryption that anything able to read the database can undo; protects a copied database only. Weaker than the secret store where one exists.

### Secret store with a file fallback (chosen)

Uses the operating system's protection where it exists and states the residual risk where it does not.
