# Architecture as built

Baley coordinates AI-assisted engineering through an MCP server and a tool guard. The running implementation is still the `cadence` binary. A separate `baley` binary and SQLite storage crates are being built, but no product path calls that store yet. The binary entry points are in [`crates/cadence/src/main.rs`](../../crates/cadence/src/main.rs) and [`crates/baley/src/main.rs`](../../crates/baley/src/main.rs).

## System context

```mermaid
flowchart LR
  owner(["Owner<br/><small>Directs and approves work</small>"])
  host["Host<br/><small>Claude Code or Codex, when configured</small>"]
  baley["Baley<br/><small>cadence runs today; baley is a CLI parser</small>"]
  repo["Git checkout<br/><small>Source and project files</small>"]
  forge["GitHub forge<br/><small>Landing when configured</small>"]
  reviewers["Review providers<br/><small>OpenAI, Gemini, DeepSeek</small>"]
  owner -->|directs work| host
  owner -->|baley help and version| baley
  host -->|MCP calls and hook input| baley
  baley -->|reads and writes .planning; runs git| repo
  baley -->|landing: gh API and git push| forge
  baley -->|review requests: HTTPS| reviewers
  linkStyle default stroke:#64748b,stroke-width:2px
  classDef person fill:#08427b,stroke:#052e56,color:#fff
  classDef system fill:#1168bd,stroke:#0b4884,color:#fff
  classDef external fill:#6b6b6b,stroke:#4d4d4d,color:#fff
  class owner person
  class baley system
  class host,repo,forge,reviewers external
```

*Figure 1. C4 system context. The live external calls come from `cadence`: git commands and GitHub landing through `gh` or `git push`, plus configured HTTPS review providers. See [`crates/cadence/src/landing/forge.rs`](../../crates/cadence/src/landing/forge.rs), [`crates/cadence/src/landing/effects.rs`](../../crates/cadence/src/landing/effects.rs), and [`crates/cadence/src/review/provider/transport.rs`](../../crates/cadence/src/review/provider/transport.rs).*

## Containers and code boundaries

```mermaid
flowchart TB
  owner(["Owner"])
  host["Host<br/><small>Claude Code or Codex, when configured</small>"]
  repo["Git checkout"]
  forge["GitHub forge"]
  reviewers["Review providers"]
  subgraph baley_system["Baley"]
    direction LR
    subgraph live["Running path"]
      direction TB
      server["cadence serve<br/><small>MCP server process</small>"]
      guard["cadence guard<br/><small>PreToolUse hook process</small>"]
      json[("project/.planning/<br/><small>JSON records and rendered files</small>")]
    end
    subgraph build1["Build 1 code: no product store caller"]
      direction TB
      cli["baley<br/><small>CLI parser only</small>"]
      core["baley-core<br/><small>registry and retention rules</small>"]
      port["baley-store<br/><small>types and storage traits</small>"]
      adapter["baley-store-sqlite<br/><small>partial SQLite adapter</small>"]
      db[("home/baley.db<br/><small>created when adapter is opened</small>")]
    end
  end
  owner -->|directs| host
  owner -->|help and version| cli
  host -->|MCP over stdio| server
  host -->|PreToolUse input| guard
  server -->|reads and writes| json
  guard -->|records Bash guard decisions| json
  server -->|git operations| repo
  guard -->|reads branch and files| repo
  server -->|landing: gh API and git push| forge
  server -->|HTTPS review requests| reviewers
  cli -.->|manifest dependency only| core
  cli -.->|manifest dependency only| adapter
  core -.->|uses types and traits| port
  adapter -.->|uses port; implements Views and Payloads| port
  adapter -.->|SqliteStore open| db
  linkStyle default stroke:#64748b,stroke-width:2px
  classDef person fill:#08427b,stroke:#052e56,color:#fff
  classDef container fill:#2e6fa5,stroke:#214f77,color:#fff
  classDef external fill:#6b6b6b,stroke:#4d4d4d,color:#fff
  class owner person
  class server,guard,cli,core,port,adapter,json,db container
  class host,repo,forge,reviewers external
  style baley_system fill:#eef4fa,stroke:#2e6295,color:#17324d
  style live fill:#ddeaf7,stroke:#2e6295,color:#17324d
  style build1 fill:#ddeaf7,stroke:#2e6295,color:#17324d
```

*Figure 2. C4 container view with the Build 1 libraries shown beside the processes. Solid arrows are running paths. Dashed arrows are source dependencies or adapter entry points; they do not connect the product to `baley.db`.*

`cadence serve` binds to a project's `.planning` directory and serves MCP over stdio; `cadence guard` reads `PreToolUse` input and can record Bash guard decisions in the same JSON store. The store names `state.json`, `decisions.jsonl`, and `items.jsonl`, and writes rendered project files there. See [`crates/cadence/src/main.rs`](../../crates/cadence/src/main.rs), [`crates/cadence/src/server.rs`](../../crates/cadence/src/server.rs), [`crates/cadence/src/guard/mod.rs`](../../crates/cadence/src/guard/mod.rs), [`crates/cadence/src/guard/bash.rs`](../../crates/cadence/src/guard/bash.rs), [`crates/cadence/src/session/mod.rs`](../../crates/cadence/src/session/mod.rs), [`crates/cadence/src/store/model.rs`](../../crates/cadence/src/store/model.rs), and [`crates/cadence/src/store/filesystem.rs`](../../crates/cadence/src/store/filesystem.rs). The checked-in hook invokes `cadence guard` for Bash, Write, and Edit in [`hooks/hooks.json`](../../hooks/hooks.json). This checkout has no `.mcp.json`; MCP host registration is not checked in.

`baley` defines an empty Clap CLI and calls only `Cli::parse()`, so it has help and version output but no store or domain command. Its manifest depends on `baley-core` and `baley-store-sqlite`; neither is called from [`crates/baley/src/main.rs`](../../crates/baley/src/main.rs) ([manifest](../../crates/baley/Cargo.toml)). `baley-core` has an event schema registry and retention rules and depends on the types and traits in `baley-store`. The SQLite crate depends on `baley-store`; it opens `<home>/baley.db` when `SqliteStore::open` is called and currently implements view and payload access plus transaction, retention, and view rebuild methods. See [`crates/baley-core/src/lib.rs`](../../crates/baley-core/src/lib.rs), [`crates/baley-store/src/ledger.rs`](../../crates/baley-store/src/ledger.rs), [`crates/baley-store-sqlite/src/store.rs`](../../crates/baley-store-sqlite/src/store.rs), [`crates/baley-store-sqlite/src/view.rs`](../../crates/baley-store-sqlite/src/view.rs), and [`crates/baley-store-sqlite/src/payload.rs`](../../crates/baley-store-sqlite/src/payload.rs).

The designed figures in [`design 0001`](../design/0001-evidence-ledger.md) show a CLI, MCP server, and guard sharing an event ledger behind the storage port, with chain heads anchored on the forge. Today the MCP server and guard still use `.planning` JSON, the new CLI has no commands, and no running path opens SQLite or publishes ledger anchors. GitHub contact in the current code is for landing work, not ledger anchoring. See [`crates/cadence/src/landing/forge.rs`](../../crates/cadence/src/landing/forge.rs), [`crates/baley/src/main.rs`](../../crates/baley/src/main.rs), and [`crates/baley-store-sqlite/src/store.rs`](../../crates/baley-store-sqlite/src/store.rs).
