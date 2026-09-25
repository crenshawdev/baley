# Architecture decision records

Each file records one architectural decision: its context, the options weighed, the choice and its consequences. The format is [MADR](https://adr.github.io/madr/), after Michael Nygard. How records are written, reviewed and superseded is described in [the design process](../design/README.md).

Start a new record from [TEMPLATE.md](TEMPLATE.md) with the next free four-digit number.

## Index

| Number | Decision | Status |
|---|---|---|
| [0001](0001-event-ledger.md) | Record evidence as an append-only, hash-chained event ledger | Proposed |
| [0002](0002-sqlite.md) | Use SQLite as the storage engine | Proposed |
| [0003](0003-per-user-database.md) | Keep one ledger database per user, outside any checkout | Proposed |
| [0004](0004-project-identity.md) | Identify projects by a committed project file | Proposed |
| [0005](0005-storage-port.md) | Put storage behind a port with engine adapters | Proposed |
| [0006](0006-no-markdown-records.md) | Keep every operational record in the ledger | Proposed |
