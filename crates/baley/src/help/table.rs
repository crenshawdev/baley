//! The shipped user commands and their shared front matter descriptions.
use serde::Serialize;
use serde_json::{Value, json};

use crate::help::front_door;
use crate::instruction::{self, Lookup};

#[derive(Serialize)]
pub struct Command {
    pub name: &'static str,
    pub cluster: &'static str,
    pub description: &'static str,
}

pub const CLUSTERS: &[&str] = &[
    "Build spine",
    "Review & quality gates",
    "Lifecycle & git",
    "Support",
];

pub const COMMANDS: &[Command] = &[
    Command {
        name: "bal-context",
        cluster: "Build spine",
        description: "Discuss a phase's scope, decisions and truths with its owner, then publish only the exact approved set through Baley",
    },
    Command {
        name: "bal-plan",
        cluster: "Build spine",
        description: "Author a phase's plans and publish the exact owner-approved content through Baley",
    },
    Command {
        name: "bal-execute",
        cluster: "Build spine",
        description: "Execute a native phase: the binary composes each executor dispatch from state and owns every task, run and suite receipt.",
    },
    Command {
        name: "bal-verify",
        cluster: "Build spine",
        description: "Inspect a phase through the retained native verifier dispatch.",
    },
    Command {
        name: "bal-progress",
        cluster: "Build spine",
        description: "Show derived phase status, located issues, records, captures and the next action.",
    },
    Command {
        name: "bal-task",
        cluster: "Build spine",
        description: "Execute a small off-roadmap task with atomic commits - inline by default, --plan for multi-step work",
    },
    Command {
        name: "bal-review",
        cluster: "Review & quality gates",
        description: "Review one explicitly selected target - a decision, a minimalism delete-list over code, or a plan - through the native review subsystem.",
    },
    Command {
        name: "bal-plan-review",
        cluster: "Review & quality gates",
        description: "Alias of /bal-review plan: review a phase's native plan slices with its locked context, or one plan document.",
    },
    Command {
        name: "bal-decision-review",
        cluster: "Review & quality gates",
        description: "Alias of /bal-review decision: refute one named decision in one named document.",
    },
    Command {
        name: "bal-minimalism-review",
        cluster: "Review & quality gates",
        description: "Alias of /bal-review minimalism: a ranked delete-list over one file, one frozen directory or one native phase range.",
    },
    Command {
        name: "bal-debug",
        cluster: "Review & quality gates",
        description: "Resume a recorded debug session, review its staged fix, and offer a configured consult at dead ends.",
    },
    Command {
        name: "bal-coverage",
        cluster: "Review & quality gates",
        description: "Read-only alias of /bal-audit: the phase-scoped requirement-to-evidence trace over the retained map and current verdicts; the test-generation arm is removed.",
    },
    Command {
        name: "bal-audit",
        cluster: "Review & quality gates",
        description: "Read-only verification audit: every requirement's phase-scoped trace to its plans, truths, evidence and current verdicts, with each broken edge named.",
    },
    Command {
        name: "bal-land",
        cluster: "Lifecycle & git",
        description: "Authorize landing steps, confirm the merge and follow ordered local cleanup.",
    },
    Command {
        name: "bal-milestone",
        cluster: "Lifecycle & git",
        description: "Close and prune a milestone, or confirm an explicit release manifest bump before landing.",
    },
    Command {
        name: "bal-undo",
        cluster: "Lifecycle & git",
        description: "Undo a phase's exact recorded commits and report retained progress.",
    },
    Command {
        name: "bal-capture",
        cluster: "Support",
        description: "Record a note, or a story candidate for the backlog, in the ledger.",
    },
    Command {
        name: "bal-help",
        cluster: "Support",
        description: "List Baley commands by cluster, or show one command and its compiled description.",
    },
    Command {
        name: "bal-spike",
        cluster: "Support",
        description: "Record risk-ordered spike criteria before experimenting, then retain observations and a bounded verdict.",
    },
    Command {
        name: "bal-suggest",
        cluster: "Support",
        description: "Show retune suggestions from retained decisions and apply only an accepted payload.",
    },
    Command {
        name: "bal-why",
        cluster: "Support",
        description: "Explain file[:line] through its git and planning history, or list a phase's journal refusals with <phase> refusals.",
    },
];

/// One help row as the answer shows it: the command, whether its instruction
/// is served now, and the build that owns it. Both come from the registry when
/// the answer is built, so the table holds them only once.
#[derive(Serialize)]
struct Row<'a> {
    #[serde(flatten)]
    command: &'a Command,
    available: bool,
    build: u32,
}

fn row(command: &Command) -> Row<'_> {
    let (available, build) = match instruction::lookup(command.name) {
        Lookup::Served { entry, .. } => (true, entry.build),
        Lookup::Unavailable { build, .. } => (false, build),
        // A test checks every row, so a command with no entry never ships.
        Lookup::Unknown => panic!("help row {} has no registry entry", command.name),
    };
    Row {
        command,
        available,
        build,
    }
}

pub fn description(name: &str) -> &'static str {
    COMMANDS
        .iter()
        .find(|row| row.name == name)
        .expect("compiled user skill")
        .description
}

/// Replace only a description value, preserving its quoted/plain form and all
/// other bytes. Also used to regenerate the seven authored skill bodies.
pub fn render_description(name: &str, markdown: &str) -> Option<String> {
    let row = COMMANDS.iter().find(|row| row.name == name)?;
    let (frontmatter, _) = markdown.strip_prefix("---\n")?.split_once("\n---\n")?;
    let marker = "\ndescription: ";
    let start = 4 + frontmatter.find(marker)? + marker.len();
    let end = start + markdown[start..].find('\n')?;
    let description = if markdown[start..end].starts_with('"') {
        serde_json::to_string(row.description).expect("description string")
    } else {
        row.description.to_owned()
    };
    Some(format!(
        "{}{description}{}",
        &markdown[..start],
        &markdown[end..]
    ))
}

/// What Baley does, in plain words, for a session that asks for help.
const ABOUT: &str = "Baley keeps AI coding agents accountable to the person who answers for their work. It records what the agents did, and the proof that it works, in a hash-chained ledger the owner controls, and it refuses to let work move on an unproven claim. Claude Code is the host it serves.";

/// The request a session follows so the ledger can say which instruction guided a write.
const REQUEST: &str = "Send `bal-help` as `instruction` on every `baley_apply` call you make while following help, and on no `baley_query` call.";

/// The help instruction's identity, version and hash, as the registry serves them.
fn help_instruction() -> Value {
    match instruction::lookup(front_door::IDENTITY) {
        Lookup::Served { entry, text } => {
            json!({"identity":entry.identity, "version":text.version, "hash":text.hash})
        }
        _ => panic!("the help instruction is served, and a test checks it"),
    }
}

pub fn answer(name: Option<&str>) -> Value {
    let Some(name) = name else {
        let clusters: Vec<_> = CLUSTERS.iter().map(|cluster| json!({
            "name": cluster,
            "commands": COMMANDS.iter().filter(|row| row.cluster == *cluster).map(row).collect::<Vec<_>>(),
        })).collect();
        return json!({"status":"ok", "clusters":clusters, "about":ABOUT,
            "instruction":help_instruction(), "request":REQUEST});
    };
    let name = name.strip_prefix('/').unwrap_or(name);
    let name = name.strip_prefix("bal-").unwrap_or(name);
    let rows: Vec<_> = COMMANDS
        .iter()
        .filter(|row| row.name.strip_prefix("bal-") == Some(name))
        .map(row)
        .collect();
    let mut closest = Vec::new();
    if rows.is_empty() {
        let mut ranked: Vec<_> = COMMANDS
            .iter()
            .map(|row| {
                (
                    edit_distance(name, row.name.strip_prefix("bal-").unwrap()),
                    row.name,
                )
            })
            .collect();
        ranked.sort_unstable();
        closest.extend(ranked.into_iter().take(3).map(|(_, name)| name));
    }
    json!({"status":"ok", "rows":rows, "closest":closest, "about":ABOUT,
        "instruction":help_instruction(), "request":REQUEST})
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<_> = right.chars().collect();
    let mut previous: Vec<_> = (0..=right.len()).collect();
    for (i, a) in left.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, b) in right.iter().enumerate() {
            current.push(
                (current[j] + 1)
                    .min(previous[j + 1] + 1)
                    .min(previous[j] + usize::from(a != *b)),
            );
        }
        previous = current;
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every help row's availability and owning build, written out from the
    /// design rather than read from the registry under test.
    const EXPECTED: &[(&str, bool, u64)] = &[
        ("bal-help", true, 3),
        ("bal-capture", true, 3),
        ("bal-context", false, 4),
        ("bal-plan", false, 4),
        ("bal-review", false, 4),
        ("bal-plan-review", false, 4),
        ("bal-decision-review", false, 4),
        ("bal-minimalism-review", false, 4),
        ("bal-execute", false, 5),
        ("bal-verify", false, 5),
        ("bal-audit", false, 5),
        ("bal-coverage", false, 5),
        ("bal-land", false, 6),
        ("bal-milestone", false, 6),
        ("bal-undo", false, 6),
        ("bal-progress", false, 7),
        ("bal-suggest", false, 7),
        ("bal-task", false, 8),
        ("bal-debug", false, 8),
        ("bal-spike", false, 8),
        ("bal-why", false, 8),
    ];

    fn triple(row: &Value) -> (String, bool, u64) {
        (
            row["name"].as_str().expect("a row name").to_owned(),
            row["available"].as_bool().expect("a row says available"),
            row["build"].as_u64().expect("a row names its build"),
        )
    }

    #[test]
    fn every_row_of_the_cluster_list_shows_the_availability_and_build_the_design_gives_it() {
        let answer = answer(None);
        let mut shown: Vec<_> = answer["clusters"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|cluster| cluster["commands"].as_array().unwrap())
            .map(triple)
            .collect();
        shown.sort();
        let mut expected: Vec<_> = EXPECTED
            .iter()
            .map(|(name, available, build)| ((*name).to_owned(), *available, *build))
            .collect();
        expected.sort();
        assert_eq!(expected.len(), 21);
        assert_eq!(shown, expected);
    }

    #[test]
    fn a_named_answer_shows_the_availability_and_build_of_its_row() {
        let plan = answer(Some("plan"));
        let rows = plan["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(triple(&rows[0]), ("bal-plan".to_owned(), false, 4));
        let help = answer(Some("/bal-help"));
        assert_eq!(
            triple(&help["rows"].as_array().unwrap()[0]),
            ("bal-help".to_owned(), true, 3)
        );
    }

    #[test]
    fn both_help_answers_carry_the_registrys_help_identity_version_and_hash() {
        let Lookup::Served { entry, text } = instruction::lookup("bal-help") else {
            panic!("bal-help is served");
        };
        for answer in [answer(None), answer(Some("plan"))] {
            let carried = &answer["instruction"];
            assert_eq!(carried["identity"], entry.identity);
            assert_eq!(carried["version"], text.version);
            assert_eq!(carried["hash"], text.hash);
        }
    }

    #[test]
    fn both_help_answers_ask_for_bal_help_as_instruction_on_baley_apply() {
        for answer in [answer(None), answer(Some("plan"))] {
            let request = answer["request"].as_str().expect("a request sentence");
            for needle in [
                "`bal-help`",
                "`instruction`",
                "`baley_apply`",
                "on no `baley_query` call",
            ] {
                assert!(request.contains(needle), "the request lost {needle}");
            }
        }
    }

    #[test]
    fn both_help_answers_say_what_baley_does_and_name_claude_code() {
        for answer in [answer(None), answer(Some("plan"))] {
            let about = answer["about"].as_str().expect("an about sentence");
            assert!(about.contains("ledger") && about.contains("Claude Code"));
        }
    }

    #[test]
    fn a_capture_row_still_offering_todos_or_seeds_or_missing_a_kind_is_caught() {
        let described = description("bal-capture");
        assert!(described.contains("note") && described.contains("story"));
        assert!(!described.contains("todo") && !described.contains("seed"));
    }

    #[test]
    fn no_help_answer_names_a_read_build_3_does_not_serve_or_a_skills_path() {
        let forbidden = [
            "`document`",
            "\"document\"",
            "document-search",
            ".planning",
            "skills/",
            "SKILL.md",
            "CLAUDE_PLUGIN_ROOT",
        ];
        for answer in [answer(None), answer(Some("plan"))] {
            let text = answer.to_string();
            for word in forbidden {
                assert!(!text.contains(word), "a help answer contains {word}");
            }
        }
    }
}
