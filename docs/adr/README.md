# Architecture decision records

Each file records one architectural decision: its context, the options weighed, the choice and its consequences. The format is [MADR](https://adr.github.io/madr/), after Michael Nygard. How records are written, reviewed and superseded is described in [the design process](../design/README.md).

Start a new record from [TEMPLATE.md](TEMPLATE.md) with the next free four-digit number.

## Index

| Number | Decision | Status |
|---|---|---|
| [0001](0001-event-ledger.md) | Record evidence as an append-only, hash-chained event ledger | Accepted |
| [0002](0002-sqlite.md) | Use SQLite as the storage engine | Accepted |
| [0003](0003-per-user-database.md) | Keep one ledger database per user, outside any checkout | Accepted |
| [0004](0004-project-identity.md) | Identify projects by a committed project file | Accepted |
| [0005](0005-storage-port.md) | Put storage behind a port with engine adapters | Accepted, superseded in part by 0010 |
| [0006](0006-no-markdown-records.md) | Keep every operational record in the ledger | Accepted |
| [0007](0007-forge-anchors.md) | Anchor chain heads on the forge | Accepted |
| [0008](0008-host-sandbox-isolation.md) | Use host sandboxes to keep agents out of the ledger | Accepted, superseded in part by 0020 |
| [0009](0009-served-instructions.md) | Serve instructions from the binary; files on disk are stubs | Accepted |
| [0010](0010-projector-traits-in-the-port.md) | Define the projector and event schema traits in the port | Accepted |
| [0011](0011-one-shared-server.md) | Run one shared Baley server per user over stdio and HTTP | Accepted |
| [0012](0012-optimistic-concurrency.md) | Use optimistic concurrency in the shared server | Accepted |
| [0013](0013-host-session-calls-outside-models.md) | Let the host session call outside models, never Baley | Accepted |
| [0014](0014-baley-runs-tests.md) | Have Baley run tests and checks itself and judge by exit code | Accepted |
| [0015](0015-settings-in-toml.md) | Keep settings in TOML: one global file, one project file, host sections | Accepted |
| [0016](0016-key-store.md) | Store provider API keys encrypted in the ledger with the master key in the OS secret store | Accepted |
| [0017](0017-stories-and-sprints.md) | Codify Scrum: a requirement is a story that carries its truths, a phase is a sprint | Accepted |
| [0018](0018-lease-enforced-at-close.md) | Enforce the lease at task close on both hosts, with the guard as an early stop where it sees writes | Accepted |
| [0019](0019-reviews-adjudicated-and-ruled.md) | Run every configured reviewer, adjudicate in the host session, and let the owner rule on each finding | Accepted |
| [0020](0020-sandbox-is-a-write-barrier.md) | State what each host's sandbox denies; reads are the host's policy | Accepted, supersedes 0008 in part |
