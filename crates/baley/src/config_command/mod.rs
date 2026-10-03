//! `baley config`: the owner's settings commands (design 0003 section 5).
//! `set` changes a settings file, `show` reports every setting and `interview`
//! asks for the model and effort of each role in the terminal. The judges
//! live in `baley_core::policy::config_command`; the files here gather what
//! they judge and print what they return.
//!
//! The module is not named `config`, which the inherited engine holds in
//! `lib.rs` and `main.rs` until Build 9.

mod interview;
mod set;
mod show;

use std::process::ExitCode;

use baley_core::policy::{FileLayer, Host};
use clap::{Args, Subcommand};

use crate::ledger::display;

/// Arguments for `baley config`.
#[derive(Args, Debug, Clone)]
pub struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Subcommand, Debug, Clone)]
enum ConfigCommand {
    /// Write one or more settings into the global file or the project file.
    Set(SetArgs),
    /// Show every setting with its layer, its diagnostics and where it was set.
    Show(ShowArgs),
    /// Ask for each role's model and effort and for `escalate_on_failure`, then write the answers.
    Interview(InterviewArgs),
}

#[derive(Args, Debug, Clone)]
struct ShowArgs {
    /// Apply this host's sections. Only claude-code is supported.
    #[arg(long, value_name = "NAME", value_parser = host)]
    host: Option<Host>,
    /// The settings to show, every setting when none is named.
    #[arg(value_name = "NAME")]
    names: Vec<String>,
}

#[derive(Args, Debug, Clone)]
struct InterviewArgs {
    #[command(flatten)]
    file: OptionalLayerFlags,
    /// Apply this host's sections to each value in force and write the answers into them. Only claude-code is supported.
    #[arg(long, value_name = "NAME", value_parser = host)]
    host: Option<Host>,
}

#[derive(Args, Debug, Clone)]
struct SetArgs {
    #[command(flatten)]
    layer: LayerFlags,
    /// Write into this host's section. Only claude-code is supported.
    #[arg(long, value_name = "NAME", value_parser = host)]
    host: Option<Host>,
    /// A setting and its value, as NAME=VALUE.
    #[arg(required = true, value_name = "NAME=VALUE", value_parser = pair)]
    pairs: Vec<(String, String)>,
}

/// Exactly one file: clap refuses both flags and neither.
#[derive(Args, Debug, Clone)]
#[group(required = true, multiple = false)]
struct LayerFlags {
    /// Write the global file, `config.toml` in the config folder.
    #[arg(long)]
    global: bool,
    /// Write the project file, `baley.toml` in the project.
    #[arg(long)]
    project: bool,
}

/// At most one file: clap refuses both flags and accepts neither.
#[derive(Args, Debug, Clone)]
#[group(required = false, multiple = false)]
struct OptionalLayerFlags {
    /// Write the global file, `config.toml` in the config folder.
    #[arg(long)]
    global: bool,
    /// Write the project file, `baley.toml` in the project.
    #[arg(long)]
    project: bool,
}

impl OptionalLayerFlags {
    /// The file asked for, `None` when neither flag is given.
    fn layer(&self) -> Option<FileLayer> {
        match (self.global, self.project) {
            (true, _) => Some(FileLayer::Global),
            (_, true) => Some(FileLayer::Project),
            _ => None,
        }
    }
}

impl LayerFlags {
    fn layer(&self) -> FileLayer {
        if self.global {
            FileLayer::Global
        } else {
            FileLayer::Project
        }
    }
}

fn host(text: &str) -> Result<Host, String> {
    Host::parse(text).ok_or_else(|| {
        let hosts: Vec<&str> = Host::ALL.iter().map(|host| host.name()).collect();
        let noun = if hosts.len() == 1 { "host" } else { "hosts" };
        format!("the supported {noun}: {}", hosts.join(", "))
    })
}

/// Splits at the first `=`, so a value may hold one.
fn pair(text: &str) -> Result<(String, String), String> {
    text.split_once('=')
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .ok_or_else(|| "expected NAME=VALUE".to_owned())
}

/// Runs one `baley config` command and prints what it did.
pub fn run(args: ConfigArgs) -> ExitCode {
    let result = match args.command {
        ConfigCommand::Set(args) => set::set(args.layer.layer(), args.host, &args.pairs),
        ConfigCommand::Show(args) => show::show(args.host, &args.names),
        ConfigCommand::Interview(args) => interview::interview(
            args.file.layer(),
            args.host,
            &mut std::io::stdin().lock(),
            &mut std::io::stdout().lock(),
        ),
    };
    ExitCode::from(display::emit(
        &result,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// The shape of `main.rs`, global `--project-root` included.
    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        command: Top,
        #[arg(long, global = true)]
        project_root: Option<std::path::PathBuf>,
    }

    #[derive(Subcommand)]
    enum Top {
        Config(ConfigArgs),
    }

    fn parse_set(args: &[&str]) -> Result<SetArgs, clap::Error> {
        let argv = ["baley", "config", "set"]
            .into_iter()
            .chain(args.iter().copied());
        let Top::Config(config) = Cli::try_parse_from(argv)?.command;
        let ConfigCommand::Set(set) = config.command else {
            panic!("not a set");
        };
        Ok(set)
    }

    fn pairs(set: &SetArgs) -> Vec<(&str, &str)> {
        set.pairs
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect()
    }

    fn parse_show(args: &[&str]) -> Result<ShowArgs, clap::Error> {
        let argv = ["baley", "config", "show"]
            .into_iter()
            .chain(args.iter().copied());
        let Top::Config(config) = Cli::try_parse_from(argv)?.command;
        let ConfigCommand::Show(show) = config.command else {
            panic!("not a show");
        };
        Ok(show)
    }

    fn parse_interview(args: &[&str]) -> Result<InterviewArgs, clap::Error> {
        let argv = ["baley", "config", "interview"]
            .into_iter()
            .chain(args.iter().copied());
        let Top::Config(config) = Cli::try_parse_from(argv)?.command;
        let ConfigCommand::Interview(interview) = config.command else {
            panic!("not an interview");
        };
        Ok(interview)
    }

    #[test]
    fn interview_alone_names_no_file_and_no_host() {
        let interview = parse_interview(&[]).unwrap();
        assert_eq!(interview.file.layer(), None);
        assert_eq!(interview.host, None);
    }

    #[test]
    fn interview_global_flag_selects_the_global_file() {
        let interview = parse_interview(&["--global"]).unwrap();
        assert_eq!(interview.file.layer(), Some(FileLayer::Global));
    }

    #[test]
    fn interview_project_flag_selects_the_project_file_not_a_clash_with_project_root() {
        let interview = parse_interview(&["--project"]).unwrap();
        assert_eq!(interview.file.layer(), Some(FileLayer::Project));
    }

    #[test]
    fn interview_with_both_file_flags_exits_2() {
        let error = parse_interview(&["--global", "--project"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn interview_takes_a_known_host_and_refuses_an_unknown_one_with_exit_2() {
        let interview = parse_interview(&["--host", "claude-code"]).unwrap();
        assert_eq!(interview.host, Some(Host::ClaudeCode));
        let error = parse_interview(&["--host", "gemini"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn show_alone_asks_for_every_setting_on_the_command_line() {
        let show = parse_show(&[]).unwrap();
        assert!(show.names.is_empty());
        assert_eq!(show.host, None);
    }

    #[test]
    fn show_keeps_the_names_asked_in_order() {
        let show = parse_show(&["roles.planner.effort", "escalate_on_failure"]).unwrap();
        assert_eq!(show.names, ["roles.planner.effort", "escalate_on_failure"]);
    }

    #[test]
    fn show_takes_a_known_host_and_refuses_an_unknown_one_with_exit_2() {
        assert_eq!(
            parse_show(&["--host", "claude-code"]).unwrap().host,
            Some(Host::ClaudeCode)
        );
        let error = parse_show(&["--host", "gemini"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn global_flag_selects_the_global_layer_with_its_pair() {
        let set = parse_set(&["--global", "a=b"]).unwrap();
        assert_eq!(set.layer.layer(), FileLayer::Global);
        assert_eq!(pairs(&set), [("a", "b")]);
    }

    #[test]
    fn project_flag_selects_the_project_layer_not_a_clash_with_project_root() {
        let set = parse_set(&["--project", "a=b"]).unwrap();
        assert_eq!(set.layer.layer(), FileLayer::Project);
        assert_eq!(pairs(&set), [("a", "b")]);
    }

    #[test]
    fn both_layer_flags_or_neither_exit_2() {
        for args in [&["--global", "--project", "a=b"][..], &["a=b"][..]] {
            let error = parse_set(args).unwrap_err();
            assert_eq!(error.exit_code(), 2, "{args:?}");
        }
    }

    #[test]
    fn a_known_host_parses_and_an_unknown_one_exits_2_naming_the_supported_host() {
        let set = parse_set(&["--global", "--host", "claude-code", "a=b"]).unwrap();
        assert_eq!(set.host, Some(Host::ClaudeCode));
        let error = parse_set(&["--global", "--host", "gemini", "a=b"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        let text = error.to_string();
        assert!(text.contains("the supported host: claude-code"), "{text}");
        assert!(!text.contains("codex"), "{text}");
    }

    #[test]
    fn a_removed_host_is_still_refused_with_exit_2_naming_the_supported_host() {
        let errors = [
            parse_set(&["--global", "--host", "codex", "a=b"]).unwrap_err(),
            parse_show(&["--host", "codex"]).unwrap_err(),
            parse_interview(&["--host", "codex"]).unwrap_err(),
        ];
        for error in errors {
            assert_eq!(error.exit_code(), 2);
            let text = error.to_string();
            assert!(text.contains("the supported host: claude-code"), "{text}");
        }
    }

    #[test]
    fn a_pair_without_equals_exits_2_and_a_value_may_hold_equals() {
        let error = parse_set(&["--global", "ab"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        let set = parse_set(&["--global", "a=b=c"]).unwrap();
        assert_eq!(pairs(&set), [("a", "b=c")]);
    }

    #[test]
    fn a_set_with_no_pair_exits_2() {
        let error = parse_set(&["--global"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }
}
