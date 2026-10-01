//! `baley config interview` (design 0003 section 5). It gathers both settings
//! files and HEAD's copy as `show` does, asks for each role's model and effort
//! and for `escalate_on_failure` in the terminal, and writes the answers
//! through one `set::set`. It opens no store and runs no policy step itself.

use baley_core::policy::config_command::judge_show;
use baley_core::policy::{
    EffectivePolicy, FileLayer, Host, Role, Schema, SettingsFile, Unavailable, merge,
};
use baley_store::StoreError;
use std::io::{self, BufRead, Write};

use super::set;
use crate::discovery::{self, Discovery, PROJECT_FILE};
use crate::folders::{Environment, Folders, Platform};
use crate::ledger::display::{self, Render};
use crate::policy_step::{self, Reads};
use crate::settings;

/// Asks the questions on `input` and `output`, then writes the answers.
/// `file` is the file the owner named, `None` when neither flag was given.
pub(crate) fn interview(
    file: Option<FileLayer>,
    host: Option<Host>,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Render {
    attempt(file, host, input, output).unwrap_or_else(|render| render)
}

fn attempt(
    file: Option<FileLayer>,
    host: Option<Host>,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Render, Render> {
    let folders = Folders::resolve(Platform::current(), &Environment::read())
        .map_err(|refusal| Render::refusal(refusal.to_string()))?;
    let unavailable =
        |error: io::Error| display::store_error(&StoreError::Unavailable(error.to_string()), None);
    let cwd = std::env::current_dir().map_err(unavailable)?;
    let ancestors = discovery::ancestors(&cwd).map_err(unavailable)?;
    let global_path = settings::global_path(&folders);

    let seen = match discovery::discover(&ancestors) {
        Discovery::Managed { folder, root } => {
            let working = settings::read(&folder.join(PROJECT_FILE));
            let head = working.as_ref().ok().and_then(Option::as_ref);
            let reads = policy_step::gather(&folders.config, &root, head);
            Seen {
                in_project: true,
                working,
                reads,
            }
        }
        Discovery::Unmanaged { .. } | Discovery::Outside => Seen {
            in_project: false,
            working: Ok(None),
            reads: Reads {
                global: settings::read(&global_path),
                head: None,
            },
        },
    };
    let start = begin(Schema::standard(), file, host, seen)?;

    let questions = questions(&start.policy);
    let outcome =
        converse(&questions, start.layer, host, input, output).map_err(|error| Render {
            lines: vec![format!(
                "the interview failed: {error}; nothing was written"
            )],
            code: 1,
            error: true,
        })?;
    Ok(match outcome {
        Outcome::Declined => Render::line("Nothing was written.", 0),
        Outcome::Unchanged => {
            Render::line("Nothing changed: every answer kept the value in force.", 0)
        }
        Outcome::Send(request) => set::set(request.layer, request.host, &request.pairs),
    })
}

/// What the gatherer found.
struct Seen {
    /// Whether the working directory is in a project.
    in_project: bool,
    /// The working-tree project file as `settings::read` returned it.
    working: Result<Option<SettingsFile>, Unavailable>,
    /// The global file and HEAD's copy of the project file.
    reads: Reads,
}

/// What the interview needs once the reads are judged.
struct Start {
    /// The policy in force, as `config show` merges it.
    policy: EffectivePolicy,
    /// The file the answers are written to.
    layer: FileLayer,
}

/// Judges the reads before the first question. A refusal is exit 2 and no
/// question is asked. HEAD's read becomes the judge's input as `show` makes
/// it: no read is no file, a copy is its layer, and a failed read stays its
/// refusal.
fn begin(
    schema: &Schema,
    file: Option<FileLayer>,
    host: Option<Host>,
    seen: Seen,
) -> Result<Start, Render> {
    let head = match seen.reads.head {
        None => Ok(None),
        Some(Ok(committed)) => Ok(committed.layer),
        Some(Err(refusal)) => Err(refusal),
    };
    let layers = judge_show(
        schema,
        &[],
        seen.in_project,
        seen.reads.global,
        seen.working,
        head,
    )
    .map_err(|refusal| Render::refusal(refusal.to_string()))?;
    Ok(Start {
        policy: merge(schema, host, layers.global.as_ref(), layers.head.as_ref()),
        layer: file.unwrap_or(FileLayer::Global),
    })
}

/// One question: a setting, the value in force and the layer it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Question {
    /// The setting's name.
    name: String,
    /// The value in force, as the owner would type it.
    value: String,
    /// The layer the value came from.
    layer: &'static str,
}

impl Question {
    fn prompt(&self) -> String {
        format!("{} is {} from {}: ", self.name, self.value, self.layer)
    }
}

/// The questions in order: model then effort for each role, then
/// `escalate_on_failure`. The list is written out and never built from the
/// schema, which later builds extend with settings that are not asked here.
fn questions(policy: &EffectivePolicy) -> Vec<Question> {
    let mut questions = Vec::new();
    for role in Role::ALL {
        let (model, source) = policy.model(role);
        questions.push(Question {
            name: format!("roles.{}.model", role.name()),
            value: model.map_or_else(
                || "absent (the host session's own model)".to_owned(),
                str::to_owned,
            ),
            layer: source.layer.name(),
        });
        let (rung, source) = policy.effort(role);
        questions.push(Question {
            name: format!("roles.{}.effort", role.name()),
            value: rung.name().to_owned(),
            layer: source.layer.name(),
        });
    }
    let (escalate, source) = policy.escalate_on_failure();
    questions.push(Question {
        name: "escalate_on_failure".to_owned(),
        value: escalate.to_string(),
        layer: source.layer.name(),
    });
    questions
}

/// What the owner answered.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Collected {
    /// Every question was answered. The pairs are the non-blank answers.
    Answers(Vec<(String, String)>),
    /// The input ended before the last answer.
    Stopped,
}

/// Writes the header and each question and reads one line per question.
/// Surrounding whitespace is trimmed, and an answer that is empty after
/// trimming keeps the value in force and gives no pair.
fn ask(
    questions: &[Question],
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<Collected> {
    writeln!(
        output,
        "Answer each question, or press Enter to keep the value in force."
    )?;
    let mut pairs = Vec::new();
    for question in questions {
        write!(output, "{}", question.prompt())?;
        output.flush()?;
        let Some(answer) = read_answer(input)? else {
            // The prompt holds the line, so end it before anything else prints.
            writeln!(output)?;
            return Ok(Collected::Stopped);
        };
        if !answer.is_empty() {
            pairs.push((question.name.clone(), answer));
        }
    }
    Ok(Collected::Answers(pairs))
}

/// One trimmed line, `None` at the end of the input.
fn read_answer(input: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_owned()))
}

/// The one `config set` an interview sends.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    /// The file to write.
    layer: FileLayer,
    /// The host section to write into, `None` for the top level.
    host: Option<Host>,
    /// The non-blank answers as name and value, in question order.
    pairs: Vec<(String, String)>,
}

/// What the interview does with the answers.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// The owner stopped, so nothing is written.
    Declined,
    /// Every answer was blank, so there is nothing to write.
    Unchanged,
    /// Write these answers through `config set`.
    Send(Request),
}

fn outcome(collected: Collected, layer: FileLayer, host: Option<Host>) -> Outcome {
    match collected {
        Collected::Stopped => Outcome::Declined,
        Collected::Answers(pairs) if pairs.is_empty() => Outcome::Unchanged,
        Collected::Answers(pairs) => Outcome::Send(Request { layer, host, pairs }),
    }
}

/// Asks every question and decides what to send.
fn converse(
    questions: &[Question],
    layer: FileLayer,
    host: Option<Host>,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<Outcome> {
    let collected = ask(questions, input, output)?;
    Ok(outcome(collected, layer, host))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::committed::Committed;
    use baley_core::policy::{Fault, parse_layer};

    const GLOBAL: &str = "/c/config.toml";
    const PROJECT: &str = "/r/baley.toml";

    fn file(path: &str, text: &str) -> SettingsFile {
        settings::file(Path::new(path), text.as_bytes().to_vec())
    }

    /// The policy `config show` would merge from the supplied file texts.
    fn policy(host: Option<Host>, global: Option<&str>, head: Option<&str>) -> EffectivePolicy {
        let schema = Schema::standard();
        let global =
            global.map(|text| parse_layer(&file(GLOBAL, text), FileLayer::Global, schema).unwrap());
        let head =
            head.map(|text| parse_layer(&file(PROJECT, text), FileLayer::Project, schema).unwrap());
        merge(schema, host, global.as_ref(), head.as_ref())
    }

    fn layers_of(questions: &[Question]) -> Vec<(&str, &str, &str)> {
        questions
            .iter()
            .map(|q| (q.name.as_str(), q.value.as_str(), q.layer))
            .collect()
    }

    fn find<'a>(questions: &'a [Question], name: &str) -> (&'a str, &'a str) {
        let question = questions.iter().find(|q| q.name == name).unwrap();
        (question.value.as_str(), question.layer)
    }

    const ABSENT: &str = "absent (the host session's own model)";

    #[test]
    fn the_questions_are_the_thirteen_in_order_with_their_defaults() {
        let asked = questions(&policy(None, None, None));

        assert_eq!(
            layers_of(&asked),
            [
                ("roles.planner.model", ABSENT, "default"),
                ("roles.planner.effort", "high", "default"),
                ("roles.analyzer.model", ABSENT, "default"),
                ("roles.analyzer.effort", "high", "default"),
                ("roles.checker.model", ABSENT, "default"),
                ("roles.checker.effort", "low", "default"),
                ("roles.executor.model", ABSENT, "default"),
                ("roles.executor.effort", "high", "default"),
                ("roles.verifier.model", ABSENT, "default"),
                ("roles.verifier.effort", "high", "default"),
                ("roles.reviewer.model", ABSENT, "default"),
                ("roles.reviewer.effort", "medium", "default"),
                ("escalate_on_failure", "false", "default"),
            ]
        );
    }

    #[test]
    fn a_value_shows_the_layer_it_came_from_global_or_project() {
        let asked = questions(&policy(
            None,
            Some("[roles.checker]\neffort = \"max\"\n"),
            Some("escalate_on_failure = true\n"),
        ));

        assert_eq!(find(&asked, "roles.checker.effort"), ("max", "global"));
        assert_eq!(find(&asked, "escalate_on_failure"), ("true", "project"));
    }

    #[test]
    fn a_host_section_applies_with_its_host_layer_and_another_host_changes_nothing() {
        let global = "[host.codex.roles.planner]\nmodel = \"sonnet\"\n\
                      [host.claude-code.roles.reviewer]\neffort = \"max\"\n";
        let head = "[host.codex]\nescalate_on_failure = true\n";

        let asked = questions(&policy(Some(Host::Codex), Some(global), Some(head)));

        assert_eq!(
            find(&asked, "roles.planner.model"),
            ("sonnet", "global-host")
        );
        assert_eq!(
            find(&asked, "escalate_on_failure"),
            ("true", "project-host")
        );
        assert_eq!(find(&asked, "roles.reviewer.effort"), ("medium", "default"));
    }

    #[test]
    fn a_setting_beyond_the_thirteen_is_not_asked() {
        let mut policy = policy(None, None, None);
        let extra = policy.settings["escalate_on_failure"].clone();
        policy.settings.insert("git.remote".to_owned(), extra);

        let asked = questions(&policy);

        assert_eq!(asked.len(), 13);
        assert!(asked.iter().all(|q| q.name != "git.remote"));
    }

    /// Runs the conversation over supplied input lines.
    fn converse_over(input: &str, host: Option<Host>) -> (Outcome, String) {
        let asked = questions(&policy(None, None, None));
        let mut output = Vec::new();
        let outcome = converse(
            &asked,
            FileLayer::Global,
            host,
            &mut input.as_bytes(),
            &mut output,
        )
        .unwrap();
        (outcome, String::from_utf8(output).unwrap())
    }

    /// `n` blank answers.
    fn blanks(n: usize) -> String {
        "\n".repeat(n)
    }

    /// Thirteen answer lines, the given (index, text) answers and blank elsewhere.
    fn answers(given: &[(usize, &str)]) -> String {
        (0..13)
            .map(|i| {
                given
                    .iter()
                    .find(|(at, _)| *at == i)
                    .map_or("", |(_, text)| *text)
            })
            .map(|text| format!("{text}\n"))
            .collect()
    }

    fn pairs_of(outcome: &Outcome) -> Vec<(&str, &str)> {
        match outcome {
            Outcome::Send(request) => request
                .pairs
                .iter()
                .map(|(n, v)| (n.as_str(), v.as_str()))
                .collect(),
            other => panic!("not a request: {other:?}"),
        }
    }

    #[test]
    fn a_blank_and_an_all_whitespace_answer_each_give_no_pair() {
        let (outcome, _) = converse_over(&answers(&[(1, "   \t "), (2, "sonnet")]), None);

        assert_eq!(pairs_of(&outcome), [("roles.analyzer.model", "sonnet")]);
    }

    #[test]
    fn an_answer_is_trimmed_before_it_is_sent() {
        let (outcome, _) = converse_over(&answers(&[(0, "  sonnet  ")]), None);

        assert_eq!(pairs_of(&outcome), [("roles.planner.model", "sonnet")]);
    }

    #[test]
    fn an_answer_equal_to_the_value_in_force_is_still_sent() {
        let (outcome, _) = converse_over(&answers(&[(1, "high")]), None);

        assert_eq!(pairs_of(&outcome), [("roles.planner.effort", "high")]);
    }

    #[test]
    fn input_that_ends_before_the_last_answer_gives_no_request() {
        let (outcome, output) = converse_over(&format!("sonnet\n{}", blanks(4)), None);

        assert_eq!(outcome, Outcome::Declined);
        assert!(output.ends_with('\n'), "{output:?}");
    }

    #[test]
    fn thirteen_blank_answers_give_nothing_to_write_not_an_empty_request() {
        let (outcome, _) = converse_over(&blanks(13), None);

        assert_eq!(outcome, Outcome::Unchanged);
    }

    #[test]
    fn two_answers_give_one_request_in_question_order_with_the_layer_and_host() {
        let (outcome, output) = converse_over(
            &answers(&[(12, "true"), (3, "max")]),
            Some(Host::ClaudeCode),
        );

        assert_eq!(
            outcome,
            Outcome::Send(Request {
                layer: FileLayer::Global,
                host: Some(Host::ClaudeCode),
                pairs: vec![
                    ("roles.analyzer.effort".to_owned(), "max".to_owned()),
                    ("escalate_on_failure".to_owned(), "true".to_owned()),
                ],
            })
        );
        assert!(
            output.contains("roles.analyzer.effort is high from default: "),
            "{output}"
        );
    }

    fn seen(global: &str, working: Option<&str>, head: Option<&str>) -> Seen {
        Seen {
            in_project: true,
            working: Ok(working.map(|text| file(PROJECT, text))),
            reads: Reads {
                global: Ok(Some(file(GLOBAL, global))),
                head: head.map(|text| {
                    Ok(Committed {
                        layer: Some(file(PROJECT, text)),
                        pending: None,
                    })
                }),
            },
        }
    }

    fn refusal_text(seen: Seen) -> (u8, String) {
        let render = begin(Schema::standard(), None, None, seen).err().unwrap();
        (render.code, render.lines.join("\n"))
    }

    #[test]
    fn the_policy_asked_from_applies_the_host_the_command_names() {
        let global = "[host.codex.roles.planner]\nmodel = \"sonnet\"\n";

        let start = begin(
            Schema::standard(),
            None,
            Some(Host::Codex),
            seen(global, None, None),
        )
        .ok()
        .unwrap();

        let asked = questions(&start.policy);
        assert_eq!(
            find(&asked, "roles.planner.model"),
            ("sonnet", "global-host")
        );
    }

    #[test]
    fn a_global_file_that_is_not_toml_is_refused_before_the_first_question() {
        let (code, text) = refusal_text(seen("[roles", None, None));

        assert_eq!(code, 2);
        assert!(text.starts_with("config-unavailable: "), "{text}");
        assert!(text.contains(GLOBAL), "{text}");
    }

    #[test]
    fn a_working_tree_file_with_a_wrong_typed_value_is_refused_naming_it() {
        let (code, text) = refusal_text(seen("", Some("escalate_on_failure = 3\n"), None));

        assert_eq!(code, 2);
        assert!(text.starts_with("config-unavailable: "), "{text}");
        assert!(text.contains(PROJECT), "{text}");
        assert!(!text.contains("HEAD"), "{text}");
    }

    #[test]
    fn a_head_copy_with_a_wrong_typed_value_is_refused_as_heads() {
        let (code, text) = refusal_text(seen("", None, Some("escalate_on_failure = 3\n")));

        assert_eq!(code, 2);
        assert!(text.contains("HEAD's copy of"), "{text}");
    }

    #[test]
    fn an_unreadable_head_read_stays_its_own_refusal() {
        let failed = Unavailable {
            path: PROJECT.into(),
            fault: Fault::Unreadable {
                cause: "HEAD's copy: git is not installed".into(),
            },
        };
        let mut seen = seen("", None, None);
        seen.reads.head = Some(Err(failed.clone()));

        let (code, text) = refusal_text(seen);

        assert_eq!((code, text), (2, failed.to_string()));
    }
}
