#[path = "config/binary.rs"]
pub mod config;
/// The inherited engine, unreached by production since the per-session server and
/// kept so its tests run. Build 9 removes it.
#[allow(dead_code)]
mod inherited;
#[cfg(test)]
mod instruction_lint;
mod instruction_surfaces;
#[path = "session/binary.rs"]
pub mod session;

use clap::{Parser, Subcommand};

/// baley: the plan/execute/verify loop, served over MCP stdio.
#[derive(Parser)]
#[command(name = "baley", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run one command with a provider key and redact its output.
    Exec(baley::exec::ExecArgs),
    /// Print a Claude Code artifact Baley renders, to standard output only.
    Artifact(baley::host_artifacts::command::ArtifactArgs),
    /// Show every setting with its layer, or change settings in either file.
    Config(baley::config_command::ConfigArgs),
    /// Tie this repository to a ledger project: write baley.toml and record project.initialized.
    Init(baley::init::InitArgs),
    /// List, add, remove and update the model names Baley accepts per host and provider.
    Models(baley::models::ModelsArgs),
    /// Check the update source and stage a newer version behind the stable path.
    Update(baley::update::command::UpdateArgs),
    /// Owner operations on the evidence ledger.
    #[command(flatten)]
    Ledger(baley::ledger::LedgerCommand),
    /// Render the query-only help front door without opening a project.
    HelpInstructions,
    /// Regenerate only a skill's description from the compiled table on stdin.
    SkillDescription { name: String },
    /// Render the debug front door without opening a project.
    DebugInstructions,
    /// Render the spike front door without opening a project.
    SpikeInstructions,
    /// Render the undo front door without opening a project.
    UndoInstructions,
    /// Render the landing front door without opening a project.
    LandInstructions,
    /// Render the milestone front door without opening a project.
    MilestoneInstructions,
    /// Render the query-only progress front door without opening a project.
    ProgressInstructions,
    /// Render the retune front door without opening a project.
    SuggestInstructions,
    /// Render the query-only why front door without opening a project.
    WhyInstructions,
    /// Render the capture front door without opening a project.
    CaptureInstructions,
    /// Render the bal-task front door and task executor contract without opening a project.
    TaskInstructions,
    /// Run the MCP stdio server.
    Serve,
    /// Answer one Claude Code pre-tool hook call for Bash, Monitor, PowerShell, Read, Grep, Glob, Write, Edit or NotebookEdit.
    Guard,
    /// Render the compiled context-role skill without opening a project.
    ContextInstructions,
    /// Render the shared read-contract skill without opening a project.
    ReadInstructions,
    /// Render the compiled authoring-only planner skill without opening a project.
    PlanInstructions,
    /// Render the compiled executor contract without opening a project.
    ExecutorInstructions {
        /// Render the bal-execute front door from the same compiled source.
        #[arg(long)]
        frontdoor: bool,
    },
    /// Render the compiled verifier contract without opening a project.
    VerifierInstructions {
        /// Render the bal-verify front door from the same compiled source.
        #[arg(long)]
        frontdoor: bool,
    },
    /// Render the merged bal-review front door, or one alias, without opening a project.
    ReviewInstructions {
        /// bal-decision-review, bal-minimalism-review or bal-plan-review.
        #[arg(long)]
        alias: Option<String>,
    },
    /// Render the read-only bal-audit front door without opening a project.
    AuditInstructions {
        /// Render the bal-coverage alias of the same read-only view.
        #[arg(long)]
        coverage: bool,
    },
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve => run_serve(),
        other => run_command(other),
    }
}

fn run_command(command: Command) -> std::process::ExitCode {
    use std::io::{Read, Write};
    let command = match command {
        Command::Ledger(command) => return baley::ledger::run(command),
        Command::Exec(args) => return baley::exec::run(args),
        Command::Artifact(args) => return baley::host_artifacts::command::run(args),
        Command::Init(args) => return baley::init::run(args),
        Command::Config(args) => return baley::config_command::run(args),
        Command::Models(args) => return baley::models::run(args),
        Command::Update(args) => return baley::update::command::run(args),
        other => other,
    };
    let arguments: Vec<&str> = match &command {
        Command::Ledger(_)
        | Command::Exec(_)
        | Command::Artifact(_)
        | Command::Init(_)
        | Command::Config(_)
        | Command::Models(_)
        | Command::Update(_) => {
            unreachable!("dispatched above")
        }
        Command::Serve => return run_serve(),
        Command::Guard => return baley::guard_hook::run(),
        Command::SkillDescription { name } => {
            let mut markdown = String::new();
            if std::io::stdin().read_to_string(&mut markdown).is_err() {
                return std::process::ExitCode::FAILURE;
            }
            let Some(rendered) = baley::help::table::render_description(name, &markdown) else {
                eprintln!("baley: unknown user skill or missing description front matter");
                return std::process::ExitCode::FAILURE;
            };
            return match std::io::stdout().lock().write_all(rendered.as_bytes()) {
                Ok(()) => std::process::ExitCode::SUCCESS,
                Err(_) => std::process::ExitCode::FAILURE,
            };
        }
        Command::HelpInstructions => vec!["help-instructions"],
        Command::SpikeInstructions => vec!["spike-instructions"],
        Command::DebugInstructions => vec!["debug-instructions"],
        Command::UndoInstructions => vec!["undo-instructions"],
        Command::LandInstructions => vec!["land-instructions"],
        Command::MilestoneInstructions => vec!["milestone-instructions"],
        Command::SuggestInstructions => vec!["suggest-instructions"],
        Command::WhyInstructions => vec!["why-instructions"],
        Command::ProgressInstructions => vec!["progress-instructions"],
        Command::CaptureInstructions => vec!["capture-instructions"],
        Command::ContextInstructions => vec!["context-instructions"],
        Command::PlanInstructions => vec!["plan-instructions"],
        Command::ReadInstructions => vec!["read-instructions"],
        Command::TaskInstructions => vec!["task-instructions"],
        Command::ExecutorInstructions { frontdoor: false } => vec!["executor-instructions"],
        Command::ExecutorInstructions { frontdoor: true } => {
            vec!["executor-instructions", "--frontdoor"]
        }
        Command::VerifierInstructions { frontdoor: false } => vec!["verifier-instructions"],
        Command::VerifierInstructions { frontdoor: true } => {
            vec!["verifier-instructions", "--frontdoor"]
        }
        Command::ReviewInstructions { alias: None } => vec!["review-instructions"],
        Command::ReviewInstructions { alias: Some(alias) } => {
            vec!["review-instructions", "--alias", alias]
        }
        Command::AuditInstructions { coverage: false } => vec!["audit-instructions"],
        Command::AuditInstructions { coverage: true } => vec!["audit-instructions", "--coverage"],
    };
    let Some(rendered) = instruction_surfaces::render(&arguments) else {
        eprintln!("baley: {} is not a review command", arguments[2]);
        return std::process::ExitCode::FAILURE;
    };
    match std::io::stdout().lock().write_all(rendered.as_bytes()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => std::process::ExitCode::FAILURE,
    }
}

/// Builds the runtime `serve` runs on. It must stay current-thread: rmcp starts
/// one task per request, and only on one thread do those tasks first run in
/// arrival order, which the session queue's first-in first-out order relies on.
fn serve_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

/// Serve one session on stdio until EOF, SIGINT or SIGTERM, then drain and
/// make the one exit checkpoint attempt.
fn run_serve() -> std::process::ExitCode {
    let runtime = serve_runtime().expect("failed to start tokio runtime");
    let clean = runtime.block_on(baley::mcp::serve::run());
    // stdin and storage can be in blocking syscalls. Never add an unbounded
    // runtime destructor wait to the drain's ten-second deadline.
    runtime.shutdown_timeout(std::time::Duration::ZERO);
    if clean {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

#[cfg(test)]
mod serve_argument_tests {
    use super::*;

    #[test]
    fn the_legacy_project_root_flag_is_a_usage_error_for_serve() {
        for input in [
            vec!["baley", "serve", "--project-root", "/x"],
            vec!["baley", "--project-root", "/x", "serve"],
        ] {
            let error = Cli::try_parse_from(input).err().expect("usage refusal");
            assert_eq!(error.exit_code(), 2);
        }
        assert!(Cli::try_parse_from(["baley", "serve"]).is_ok());
    }

    #[test]
    fn the_cli_names_the_guard_and_serve_subcommands() {
        let command = |word| crate::Cli::try_parse_from(["baley", word]).unwrap().command;
        assert!(matches!(command("guard"), crate::Command::Guard));
        assert!(matches!(command("serve"), crate::Command::Serve));
    }

    #[test]
    fn serve_runs_on_a_current_thread_runtime_so_calls_reach_admission_in_order() {
        let runtime = serve_runtime().unwrap();
        assert_eq!(
            runtime.handle().runtime_flavor(),
            tokio::runtime::RuntimeFlavor::CurrentThread
        );
    }
}

#[cfg(test)]
mod update_argument_tests {
    use super::*;
    use baley::update::command::{UpdateArgs, UpdateCommand};

    #[test]
    fn an_update_subcommand_misread_or_the_seed_step_shown_is_caught() {
        let manual = Cli::try_parse_from(["baley", "update"]).unwrap();
        assert!(matches!(
            manual.command,
            Command::Update(UpdateArgs { command: None })
        ));
        let seed = Cli::try_parse_from(["baley", "update", "seed"]).unwrap();
        assert!(matches!(
            seed.command,
            Command::Update(UpdateArgs {
                command: Some(UpdateCommand::Seed)
            })
        ));
        let error = Cli::try_parse_from(["baley", "update", "--source", "x"])
            .err()
            .expect("an unknown flag must be refused");
        assert_eq!(error.exit_code(), 2);
        let help = Cli::try_parse_from(["baley", "update", "--help"])
            .err()
            .expect("long help");
        assert_eq!(help.kind(), clap::error::ErrorKind::DisplayHelp);
        assert!(!help.to_string().contains("seed"), "{help}");
    }
}

#[cfg(test)]
mod ledger_argument_tests {
    use super::*;
    #[test]
    fn verify_reaches_the_flattened_ledger_surface() {
        let cli = Cli::try_parse_from(["baley", "verify", "P", "--local-only"]).unwrap();
        assert!(
            matches!(cli.command,Command::Ledger(baley::ledger::LedgerCommand::Verify { project,local_only:true,.. }) if project.as_deref() == Some("P"))
        );
    }
}

#[cfg(test)]
mod exec_argument_tests {
    use super::*;

    #[test]
    fn exec_arguments_after_the_separator_belong_to_the_command() {
        let cli = Cli::try_parse_from([
            "baley", "exec", "--key", "A", "--", "cmd", "--key", "B", "--",
        ])
        .unwrap();
        let Command::Exec(args) = cli.command else {
            panic!("exec command lost")
        };
        assert_eq!(args.key, "A");
        assert_eq!(args.command, ["cmd", "--key", "B", "--"]);
        let cli = Cli::try_parse_from(["baley", "exec", "--key=A", "--", "ls"]).unwrap();
        let Command::Exec(args) = cli.command else {
            panic!("exec command lost")
        };
        assert_eq!(args.key, "A");
        assert_eq!(args.command, ["ls"]);
    }

    #[test]
    fn exec_refuses_a_missing_key_separator_or_command() {
        for input in [
            vec!["baley", "exec", "--key", "A", "ls"],
            vec!["baley", "exec", "--key", "A", "--"],
            vec!["baley", "exec", "--", "ls"],
            vec!["baley", "exec", "--key", "A", "--key", "B", "--", "ls"],
        ] {
            let error = Cli::try_parse_from(input).err().expect("usage refusal");
            assert_eq!(error.exit_code(), 2);
        }
    }
}

#[cfg(test)]
mod artifact_argument_tests {
    use baley::host_artifacts::command::Request;

    use super::*;

    fn request(input: &[&str]) -> Request {
        let Command::Artifact(args) = Cli::try_parse_from(input).unwrap().command else {
            panic!("artifact command lost")
        };
        args.request
    }

    #[test]
    fn an_artifact_request_lost_or_misread_on_the_way_to_the_command_is_caught() {
        assert_eq!(
            request(&["baley", "artifact", "stub", "bal-help"]),
            Request::Stub {
                identity: "bal-help".to_owned()
            }
        );
        assert_eq!(
            request(&[
                "baley",
                "artifact",
                "registration",
                "--executable",
                "/x/baley",
                "--always-load",
            ]),
            Request::Registration {
                executable: "/x/baley".into(),
                always_load: true,
            }
        );
    }

    #[test]
    fn artifact_settings_without_a_home_folder_accepted_is_caught() {
        let error = Cli::try_parse_from([
            "baley",
            "artifact",
            "settings",
            "--executable",
            "/x/baley",
            "--config",
            "/c",
        ])
        .err()
        .expect("usage refusal");
        assert_eq!(error.exit_code(), 2);
    }
}
