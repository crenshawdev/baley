workspace "Baley" "The C4 model behind Baley's design documents. Every structure diagram in docs/design is exported from this file." {

    model {
        owner = person "Owner" "The person responsible for the work. Approves plans, rules on findings, sets policy."

        baley = softwareSystem "Baley" "Decides and orchestrates every step of the process; keeps the record." {
            binary = container "Baley server" "The Rust binary. Each Claude Code session starts its own process of it over stdio as an MCP server. The command line runs it as a process of its own, and the guard hook as one process per tool call, which records its answers in the per-user ledger." "Rust" {
                hostInterface = component "Host interface" "MCP server (stdio), command line and guard hook: the only ways in. The guard hook records its answers in the per-user ledger."
                hardin = component "Hardin" "Derives the state of the work from the record, answers what may happen next and refuses the rest."
                domain = component "Domain areas" "The rules of each process area: planning, execution, verification, review, risk, landing and the rest."
                composer = component "Work order composer" "Builds every dispatch: role, model and effort from policy, instructions from the binary, inputs from the record."
                policy = component "Policy" "Reads the global and project settings, resolves the values in effect and records which applied."
                catalog = component "Model catalog" "The models each host and provider offers, seeded from the binary, changed by the owner and refreshed from imported lists."
                ports = component "Ports and adapters" "Storage, git and test runner, forge and host adapters. The core sees only these ports."
            }
            updater = container "Detached updater" "A separate process of the binary, started by baley serve only when updates.auto is on. Opens the per-user store to claim and record at most one check per UTC day per installation. Foreground manual checks share its installation scope and daily intent but never refuse as not due." "Rust"
            ledger = container "Ledger" "One append-only, hash-chained record per user, outside any checkout." "SQLite" "Database"
            settings = container "Settings" "One global file and one file per project." "TOML" "File"
        }

        host = softwareSystem "Host" "Claude Code: the owner's session, which relays Baley's work orders and adjudicates. The session's workers are its subagents, and they run inside Claude Code's sandbox." "External"
        repo = softwareSystem "Repository" "The project's git checkout." "External"
        releases = softwareSystem "Release source" "Development artifacts: a plain executable and an unsigned two-line manifest of version and SHA-256. #14 supplies signed checksum manifests and release verification." "External"
        forge = softwareSystem "Forge" "GitHub: chain anchors, pull requests, issues." "External"
        reviewers = softwareSystem "Outside reviewers" "OpenAI and DeepSeek, reached by the session through their APIs." "External"

        owner -> host "Works in"
        owner -> hostInterface "Uses the command line"
        host -> hostInterface "Work orders, results, questions" "MCP over stdio"
        host -> hostInterface "Asks before each tool call runs" "Pre-tool hook, one process per tool call"
        # Explicit, so the context and container views show the hook beside
        # MCP; the implied edges would carry only the first component relation.
        host -> baley "Asks before each tool call runs" "Pre-tool hook, one process per tool call"
        host -> binary "Asks before each tool call runs" "Pre-tool hook, one process per tool call"
        host -> repo "Workers edit source and commit"
        host -> reviewers "Review and model-list API calls with the owner's environment keys"
        hostInterface -> hardin "Asks what may happen next"
        hardin -> domain "Applies the area's rules"
        domain -> composer "Requests work orders"
        composer -> policy "Resolves role, model and effort"
        # Explicit, so the container view shows the write beside the reads; the
        # implied edge would carry only the first component relation.
        binary -> settings "Reads, and writes a file whole for config set"
        policy -> settings "Reads"
        hostInterface -> settings "Writes a settings file whole, for config set"
        policy -> catalog "Checks model names"
        policy -> ports "Records the effective policy and each route"
        hostInterface -> policy "Settings commands"
        hostInterface -> catalog "Model commands"
        catalog -> ports "Records seeds, owner changes and detections"
        domain -> ports "Records and acts through"
        ports -> ledger "Appends events, reads views"
        ports -> repo "Reads git facts, runs tests and git"
        ports -> forge "Pushes anchors, opens pull requests and issues"
        hostInterface -> updater "Starts opted-in checks at server start without waiting for network"
        updater -> ledger "Opens the per-user store through the store port, claims and records checks"
        # Keep both update paths visible in the system context.
        baley -> releases "Fetches releases verified by #14, unsigned development artifacts only in T14 and T17" "HTTPS, reqwest"
        hostInterface -> releases "Runs baley update in the foreground, waits for releases and receipt" "HTTPS, reqwest"
        updater -> releases "Fetches releases verified by #14, unsigned development artifacts only in T14 and T17" "HTTPS, reqwest"
    }

    views {
        systemContext baley "context" {
            include *
            include reviewers
            autolayout lr
        }

        container baley "containers" {
            include *
            include reviewers
            autolayout lr
        }

        component binary "components" {
            include *
            autolayout lr
        }

        component binary "configuration" {
            include hostInterface composer policy catalog ports settings ledger reviewers owner host
            autolayout lr
        }

        styles {
            element "Element" {
                color #ffffff
            }
            element "Person" {
                shape Person
                background #08427b
                stroke #052e56
            }
            element "Software System" {
                background #1168bd
                stroke #0b4884
            }
            element "Container" {
                background #438dd5
                stroke #2e6295
            }
            element "Component" {
                background #85bbf0
                stroke #5d82a8
                color #000000
            }
            element "Database" {
                shape Cylinder
            }
            element "File" {
                shape Folder
            }
            element "External" {
                background #6b6b6b
                stroke #4d4d4d
            }
            relationship "Relationship" {
                thickness 2
            }
        }
    }
}
