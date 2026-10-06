# Architecture decision records

Each file records one architectural decision: its context, the options weighed, the choice and its consequences. The format is [MADR](https://adr.github.io/madr/), after Michael Nygard. How records are written, reviewed and superseded is described in [the design process](../design/README.md).

Start a new record from [TEMPLATE.md](TEMPLATE.md) with the next free four-digit number, skipping any number the index reserves. A reservation holds a number for a decision whose record is not written yet. Its row gains its link when its record lands.

## Index

| Number | Decision | Status |
|---|---|---|
| [0001](0001-event-ledger.md) | Record evidence as an append-only, hash-chained event ledger | Accepted |
| [0002](0002-sqlite.md) | Use SQLite as the storage engine | Accepted, superseded in part by 0027 |
| [0003](0003-per-user-database.md) | Keep one ledger database per user, outside any checkout | Accepted, superseded in part by 0023 and 0027 |
| [0004](0004-project-identity.md) | Identify projects by a committed project file | Accepted |
| [0005](0005-storage-port.md) | Put storage behind a port with engine adapters | Accepted, superseded in part by 0010 |
| [0006](0006-no-markdown-records.md) | Keep every operational record in the ledger | Accepted |
| [0007](0007-forge-anchors.md) | Anchor chain heads on the forge | Accepted, superseded in part by 0026 |
| [0008](0008-host-sandbox-isolation.md) | Use host sandboxes to keep agents out of the ledger | Accepted, superseded in part by 0020 and 0033 |
| [0009](0009-served-instructions.md) | Serve instructions from the binary; files on disk are stubs | Accepted |
| [0010](0010-projector-traits-in-the-port.md) | Define the projector and event schema traits in the port | Accepted, superseded in part by 0021 |
| [0011](0011-one-shared-server.md) | Run one shared Baley server per user over stdio and HTTP | Superseded by 0034 |
| [0012](0012-optimistic-concurrency.md) | Use optimistic concurrency in the shared server | Accepted |
| [0013](0013-host-session-calls-outside-models.md) | Let the host session call outside models, never Baley | Accepted, superseded in part by 0027 |
| [0014](0014-baley-runs-tests.md) | Have Baley run tests and checks itself and judge by exit code | Accepted |
| [0015](0015-settings-in-toml.md) | Keep settings in TOML: one global file, one project file, host sections | Accepted, superseded in part by 0027 |
| [0016](0016-key-store.md) | Store provider API keys encrypted in the ledger with the master key in the OS secret store | Superseded by 0027, and in part by 0023 |
| [0017](0017-stories-and-sprints.md) | Codify Scrum: a requirement is a story that carries its truths, a phase is a sprint | Accepted, superseded in part by 0031 |
| [0018](0018-lease-enforced-at-close.md) | Enforce the lease at task close on both hosts, with the guard as an early stop where it sees writes | Accepted, superseded in part by 0033 |
| [0019](0019-reviews-adjudicated-and-ruled.md) | Run every configured reviewer, adjudicate in the host session, and let the owner rule on each finding | Accepted |
| [0020](0020-sandbox-is-a-write-barrier.md) | State what each host's sandbox denies; reads are the host's policy | Accepted, supersedes 0008 in part, superseded in part by 0027 and 0033 |
| [0021](0021-claim-rules-in-the-port.md) | Claim liveness and scope rules in the port | Accepted |
| [0022](0022-acknowledged-restore.md) | Report owner-acknowledged restores behind a remote anchor | Accepted, superseded in part by 0035 |
| [0023](0023-no-backups-in-baley.md) | Keep whole-store backups outside Baley | Accepted, supersedes 0003 and 0016 in part |
| [0024](0024-conformance-suite-and-adapter-tests.md) | Separate port conformance from adapter mechanism tests | Accepted |
| [0025](0025-anchor-tag-objects.md) | Point anchor tags at the empty tree | Accepted |
| [0026](0026-anchors-read-by-baley.md) | Anchors are read by Baley, and a missing tag ruleset is reported | Accepted, supersedes 0007 in part |
| [0027](0027-vendor-folders-and-plain-keys.md) | Keep Baley's files in its own crenshawdev folders, with provider keys in a plain keys.env | Accepted, supersedes 0016, and 0002, 0003, 0013, 0015 and 0020 in part, superseded in part by 0032 and 0033 |
| [0028](0028-one-http-stack.md) | Use one HTTP stack on tokio and hyper: reqwest for outgoing calls, axum for the MCP server | Accepted, superseded in part by 0034 |
| [0029](0029-a-host-may-offer-more.md) | Let a host offer more than the floor | Accepted, superseded in part by 0033 |
| [0030](0030-question-rounds.md) | Put refinement and planning decisions to the owner in dependency-ordered question rounds | Accepted |
| [0031](0031-one-term-per-concept.md) | Use one term per concept, kept in a glossary: phase, not sprint, and story, not requirement | Accepted, supersedes 0017 in part |
| [0032](0032-gemini-is-not-a-provider.md) | Drop Gemini from the model catalog and detection | Accepted, supersedes 0027 in part |
| [0033](0033-host-security-bar.md) | Support only hosts whose sandboxing and execution controls meet Baley's requirements | Accepted, supersedes 0008, 0018, 0020, 0027 and 0029 in part |
| [0034](0034-one-server-per-session.md) | Run one Baley server per session over stdio | Accepted, supersedes 0011, and 0028 in part |
| [0035](0035-restore-purge-uncertainty.md) | Report purge uncertainty after restoring a store | Accepted, supersedes 0022 in part |
| [0036](0036-per-user-guard-records.md) | Keep guard records per user and bound the guard's access | Accepted |
| [0037](0037-plan-recheck-scope.md) | Choose plan re-check scope explicitly | Accepted |
