workspace "Baley" "The C4 model behind Baley's design documents. Every structure diagram in docs/design is exported from this file." {

    model {
        owner = person "Owner" "The person responsible for the work. Approves plans, rules on findings, sets policy."

        baley = softwareSystem "Baley" "Decides and orchestrates every step of the process; keeps the record." {
            binary = container "Baley server" "One shared process per user. MCP server, command line and guard hook." "Rust" {
                hostInterface = component "Host interface" "MCP server (stdio and HTTP), command line and guard hook: the only ways in."
                hardin = component "Hardin" "Derives the state of the work from the record, answers what may happen next and refuses the rest."
                domain = component "Domain areas" "The rules of each process area: planning, execution, verification, review, risk, landing and the rest."
                composer = component "Work order composer" "Builds every dispatch: role, model and effort from policy, instructions from the binary, inputs from the record."
                policy = component "Policy" "Reads the global and project settings, resolves the values in effect and records which applied."
                ports = component "Ports and adapters" "Storage, git and test runner, forge and host adapters. The core sees only these ports."
            }
            ledger = container "Ledger" "One append-only, hash-chained record per user, outside any checkout." "SQLite" "Database"
            settings = container "Settings" "One global file and one file per project." "TOML" "File"
        }

        host = softwareSystem "Host" "Claude Code or Codex: the owner's session, which relays Baley's work orders and adjudicates, and the worker agents it launches." "External"
        repo = softwareSystem "Repository" "The project's git checkout." "External"
        forge = softwareSystem "Forge" "GitHub: chain anchors, pull requests, issues." "External"
        reviewers = softwareSystem "Outside reviewers" "Model providers such as OpenAI, Gemini and DeepSeek." "External"

        owner -> host "Works in"
        owner -> hostInterface "Uses the command line"
        host -> hostInterface "Work orders, results, questions" "MCP over stdio or HTTP"
        host -> repo "Workers edit source and commit"
        host -> reviewers "Outside review calls, with prompts built by Baley"
        hostInterface -> hardin "Asks what may happen next"
        hardin -> domain "Applies the area's rules"
        domain -> composer "Requests work orders"
        composer -> policy "Resolves role, model and effort"
        policy -> settings "Reads"
        domain -> ports "Records and acts through"
        ports -> ledger "Appends events, reads views"
        ports -> repo "Reads git facts, runs tests and git"
        ports -> forge "Pushes anchors, opens pull requests and issues"
    }

    views {
        systemContext baley "context" {
            include *
            autolayout lr
        }

        container baley "containers" {
            include *
            autolayout lr
        }

        component binary "components" {
            include *
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
