use serde_json::json;

use super::*;

/// The eighteen front doors not served yet, with their owning builds, written out from the
/// design rather than read from the table under test.
const UNAVAILABLE: &[(&str, u32)] = &[
    ("bal-plan", 4),
    ("bal-review", 4),
    ("bal-plan-review", 4),
    ("bal-decision-review", 4),
    ("bal-minimalism-review", 4),
    ("bal-execute", 5),
    ("bal-verify", 5),
    ("bal-audit", 5),
    ("bal-coverage", 5),
    ("bal-land", 6),
    ("bal-milestone", 6),
    ("bal-undo", 6),
    ("bal-progress", 7),
    ("bal-suggest", 7),
    ("bal-task", 8),
    ("bal-debug", 8),
    ("bal-spike", 8),
    ("bal-why", 8),
];

#[test]
fn an_unavailable_front_door_is_never_presented_as_usable_and_names_its_owning_build() {
    assert_eq!(UNAVAILABLE.len(), 18);
    for (identity, build) in UNAVAILABLE {
        assert_eq!(
            lookup(identity),
            Lookup::Unavailable {
                identity,
                build: *build
            },
            "{identity}"
        );
    }
    let unserved = ENTRIES.iter().filter(|entry| entry.text.is_none()).count();
    assert_eq!(unserved, UNAVAILABLE.len());
}

#[test]
fn the_unavailable_refusal_names_the_build_in_words_and_as_an_integer_in_slot_identity() {
    let refusal = unavailable("bal-plan", 4);
    assert_eq!(refusal["status"], "refused");
    assert_eq!(refusal["code"], "instruction-unavailable");
    assert_eq!(refusal["slot"], "identity");
    assert_eq!(refusal["details"], json!({"build": 4}));
    let reason = refusal["reason"].as_str().unwrap();
    assert!(reason.contains("Build 4") && reason.contains("bal-plan"));
}

#[test]
fn a_path_shaped_identity_is_unknown_and_no_file_content_reaches_the_answer() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("bal-help");
    let skill = dir.path().join("skills/bal-help/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&plain, "MARKER-PLAIN-FILE\n").unwrap();
    std::fs::write(&skill, "MARKER-SKILL-FILE\n").unwrap();
    let spellings = [
        plain.to_str().unwrap(),
        skill.to_str().unwrap(),
        "skills/bal-help/SKILL.md",
    ];
    for spelling in spellings {
        assert_eq!(lookup(spelling), Lookup::Unknown, "{spelling}");
    }
    let refusal = unknown().to_string();
    assert!(!refusal.contains("MARKER"));
    assert!(refusal.contains("unknown-instruction") && refusal.contains("identity"));
}

#[test]
fn a_loose_spelling_of_a_registered_identity_is_unknown() {
    assert_eq!(lookup("BAL-PLAN"), Lookup::Unknown);
    assert_eq!(lookup("/bal-plan"), Lookup::Unknown);
    assert_eq!(lookup("bal-plan "), Lookup::Unknown);
    assert_eq!(lookup(""), Lookup::Unknown);
}

/// Every (identity, version, hash) this registry has served, written out by
/// hand. Rows are only ever added: a changed text gets a new version and a new
/// row, and the old row stays so the old pair is never reused.
const SERVED_EVER: &[(&str, &str, &str)] = &[
    (
        "bal-help",
        "1",
        "6a546a62197f3070cdfa4725732e5f8e3c00d21addf3b77696459c0dcb469cad",
    ),
    (
        "bal-read-contract",
        "1",
        "4feb8474b44d0f4b85290797064cc1e57a369f5b3afdc0ca866afe8ea7d6c217",
    ),
    (
        "bal-capture",
        "1",
        "52867052fb5ae23a0962120f07e6fbfa3ba0010bf2e52c109351ff8a1e8b7c5e",
    ),
    (
        "bal-read-contract",
        "2",
        "e20a4e61b36b0e0787a672c58558b04102a4357d428155ac3b618243b578ddd2",
    ),
    (
        "bal-start-planner",
        "1",
        "5c2872a582d0d1b38105c8ff971cbeae8727d0a74e51593e80b794a2bc90488a",
    ),
    (
        "bal-start-survey",
        "1",
        "ef390f27faddc98daf28a40e86a917589caf0ae1fd8185a9ebcce06592986955",
    ),
    (
        "bal-project",
        "1",
        "7153c55fcb0acaf2712762b305b9d96cfd6a48ae4e44aa354b3685820411d3d1",
    ),
    (
        "bal-scope",
        "1",
        "8ca51542feb46f946b787f5609c6a896ee5cd7954655c9ba7397a709390e74c8",
    ),
    (
        "bal-phase",
        "1",
        "8232b5d90eea73bef1d92f09919ce23ec8dae6dec60bd76b75dca6c65c09ba3b",
    ),
    (
        "bal-story",
        "1",
        "7688ddacbf34f6c93a4eb82b8eb60d69089eb2fee0ab5176097a4dc560877693",
    ),
    (
        "bal-backlog",
        "1",
        "4dceb6272fd0d130c5d74c904919128e51a06eab7330fb7bc02b932aa3322351",
    ),
    (
        "bal-help",
        "2",
        "ab97190b91b54fb647853049e3dbf54e82d736a31ba36996a83bc5d981471733",
    ),
    (
        "bal-rounds",
        "1",
        "8bab77d68c4253a45e63cf5f16397e58e058c21f94781674c0cf08c09bfa3e6d",
    ),
    (
        "bal-analyzer-questions",
        "1",
        "bf73b552ba3f4247d11f2ac8897f12065de8f9e63f065619e4837317a293a937",
    ),
    (
        "bal-analyzer-drafting",
        "1",
        "8cfeef1bdc7915b39f5cbe2002a89db30e2a333b678af2dc468cde794bac0810",
    ),
    (
        "bal-help",
        "3",
        "31ecd5654bebed4db300e611dc9bb8dfa4cd2993c79e251bef681ed6495e3f5a",
    ),
    (
        "bal-rounds",
        "2",
        "420c5ece69f5f29e60337a0ab905db6a2c00439f77b7422290062a0fd358bb66",
    ),
    (
        "bal-context",
        "1",
        "d318a66c27dae8d904b1bf54790a526656da09ed38e5b4b2a126b5a5b2392b4c",
    ),
    (
        "bal-context",
        "2",
        "b1d0dae1c7c4da8169b407ae0ea2fdea8476545eb6ebaba08ecf496c8f564b77",
    ),
    (
        "bal-analyzer-drafting",
        "2",
        "0dad9202903bc4ee1ba81792681c893b3abe5a098cb950fe40135d9fa74ed116",
    ),
    (
        "bal-phase-planner",
        "1",
        "65a03cb2fcbee8ff52445f5363a51054404085166b9ff2e285117f084315d8c6",
    ),
    (
        "bal-phase-planner",
        "2",
        "2cfbbeb4b7f766a275204f5ac3bbbad9ff286e7d209bb4d7cb2ddbc3f90210f9",
    ),
];

fn served() -> impl Iterator<Item = (&'static str, &'static Text)> {
    ENTRIES
        .iter()
        .filter_map(|entry| entry.text.as_ref().map(|text| (entry.identity, text)))
}

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn a_served_text_changed_without_its_pinned_hash_or_a_new_version_row_fails() {
    assert!(served().count() >= 1);
    for (identity, text) in served() {
        assert_eq!(
            sha256_hex(text.body),
            text.hash,
            "{identity}: the text no longer matches its pinned hash"
        );
        assert!(
            SERVED_EVER.contains(&(identity, text.version, text.hash)),
            "{identity} version {} hash {} is not in the served-ever list",
            text.version,
            text.hash
        );
    }
}

#[test]
fn one_identity_and_version_never_carry_two_hashes_in_the_served_ever_list() {
    for (index, (identity, version, hash)) in SERVED_EVER.iter().enumerate() {
        for (other_identity, other_version, other_hash) in &SERVED_EVER[index + 1..] {
            if identity == other_identity && version == other_version {
                assert_eq!(hash, other_hash, "{identity} {version} has two hashes");
            }
        }
    }
}

#[test]
fn no_served_text_carries_inherited_lifecycle_or_disk_skill_wording() {
    let forbidden = [
        "document-search",
        ".planning",
        "skills/",
        "SKILL.md",
        "CLAUDE_PLUGIN_ROOT",
    ];
    for (identity, text) in served() {
        for word in forbidden {
            assert!(!text.body.contains(word), "{identity} contains {word}");
        }
    }
}

#[test]
fn the_help_text_asks_for_its_identity_on_apply_calls_only() {
    let Lookup::Served { text, .. } = lookup("bal-help") else {
        panic!("bal-help is served");
    };
    let body = text.body;
    assert!(body.contains("`bal-help`") && body.contains("`instruction`"));
    assert!(body.contains("Never send it on a `baley_query` call"));
}

#[test]
fn a_capture_text_without_its_identity_request_or_either_kind_is_caught() {
    let Lookup::Served { text, .. } = lookup("bal-capture") else {
        panic!("bal-capture is served");
    };
    let body = text.body;
    assert!(body.contains("`baley_apply`"));
    assert!(body.contains(r#""operation":"capture""#));
    assert!(body.contains(r#""instruction":"bal-capture""#));
    assert!(body.contains(r#""kind":"note""#) && body.contains(r#""kind":"story""#));
}

#[test]
fn a_read_contract_that_never_says_how_to_read_a_capture_back_is_caught() {
    let Lookup::Served { text, .. } = lookup("bal-read-contract") else {
        panic!("bal-read-contract is served");
    };
    assert!(text.body.contains(r#""operation":"document""#));
    assert!(text.body.contains(r#""kind":"capture""#));
}

#[test]
fn the_read_contract_stays_within_a_description_and_keeps_its_paging_and_host_tools_rules() {
    let Lookup::Served { text, .. } = lookup("bal-read-contract") else {
        panic!("bal-read-contract is served");
    };
    let body = text.body;
    assert!(
        body.chars().count() <= 2048,
        "{} characters",
        body.chars().count()
    );
    for needle in [
        "\"operation\":\"instruction\"",
        "\"identity\"",
        "part",
        "next",
        "host's own",
    ] {
        assert!(body.contains(needle), "the read contract lost {needle}");
    }
}

#[test]
fn the_help_evidence_takes_its_version_and_hash_from_the_registry_not_the_binary() {
    let evidence = evidence("bal-help").unwrap();
    // The list is append-only, so the newest row is the last one for the identity.
    let (_, version, hash) = SERVED_EVER
        .iter()
        .rfind(|(identity, _, _)| *identity == "bal-help")
        .unwrap();
    assert_eq!(evidence.identity(), "bal-help");
    assert_eq!(evidence.version(), *version);
    assert_eq!(evidence.hash(), *hash);
    assert_ne!(evidence.version(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn every_served_entry_fits_the_caller_records_limits_as_evidence() {
    for (identity, text) in served() {
        let evidence = evidence(identity).unwrap_or_else(|_| panic!("{identity} is refused"));
        assert_eq!(evidence.identity(), identity);
        assert_eq!(evidence.version(), text.version);
        assert_eq!(evidence.hash(), text.hash);
    }
}

#[test]
fn an_unavailable_or_unknown_identity_is_refused_and_given_no_evidence() {
    assert_eq!(
        evidence("bal-plan").unwrap_err(),
        unavailable("bal-plan", 4)
    );
    assert_eq!(evidence("no-such-instruction").unwrap_err(), unknown());
}

fn served_text(identity: &str) -> &'static str {
    let Lookup::Served { text, .. } = lookup(identity) else {
        panic!("{identity} is served");
    };
    text.body
}

#[test]
fn a_start_planner_text_missing_its_order_binding_or_submission_shape_is_caught() {
    let body = served_text("bal-start-planner");
    for needle in [
        "inputs",
        "brief",
        "scope-submit",
        "\"work_order\"",
        "request_id",
        "bal-start-survey",
        "baley scope approve",
        "description",
        "what",
        "core_value",
        "constraints",
        "stories",
        "label",
        "sentence",
        "status",
        "active",
        "deferred",
        "excluded",
        "roadmap",
        "name",
        "goal",
        "detail",
        "depends_on",
    ] {
        assert!(
            body.contains(needle),
            "the start planner text lacks {needle}"
        );
    }
    assert!(!body.contains("scope-approve"));
}

#[test]
fn a_survey_text_that_writes_or_submits_is_caught() {
    let body = served_text("bal-start-survey");
    assert!(body.contains("host's own file, search and shell tools"));
    assert!(body.contains("Write no file") && body.contains("make no commit"));
    assert!(!body.contains("scope-submit"));
}

#[test]
fn a_start_instruction_given_a_help_row_or_a_skill_stub_is_caught() {
    let doors = crate::host_artifacts::stubs::front_doors();
    for identity in ["bal-start-planner", "bal-start-survey"] {
        let Lookup::Served { entry, text } = lookup(identity) else {
            panic!("{identity} is served");
        };
        assert_eq!(entry.build, 4, "{identity}");
        assert_eq!(text.version, "1", "{identity}");
        assert!(
            !crate::help::table::COMMANDS
                .iter()
                .any(|command| command.name == identity),
            "{identity} is a help row"
        );
        assert!(
            !doors.iter().any(|(name, _)| *name == identity),
            "{identity} is a front door"
        );
    }
}

#[test]
fn a_project_front_door_missing_an_operation_it_drives_or_the_terminal_approval_is_caught() {
    let body = served_text("bal-project");
    for needle in [
        "project-start",
        "brief",
        r#""instruction":"bal-project""#,
        "settings",
        "start-order-pending",
        "launch",
        "prompt",
        "work-order-fail",
        "work-order-retry",
        "project-describe",
        "baley scope approve",
    ] {
        assert!(
            body.contains(needle),
            "the project front door lacks {needle}"
        );
    }
    assert!(!body.contains("scope-approve"));
}

/// The four scope change front doors, each with the operations it drives,
/// written out from design 0004 rather than read from the texts under test.
const CHANGE_DOORS: &[(&str, &[&str])] = &[
    ("bal-scope", &[r#""kind":"draft""#]),
    (
        "bal-phase",
        &[
            "phase-declare",
            "phase-edit",
            "phase-reorder",
            "phase-withdraw",
        ],
    ),
    (
        "bal-story",
        &[
            "story-declare",
            "story-edit",
            "story-reassign",
            "story-drop",
        ],
    ),
    ("bal-backlog", &["backlog-reorder"]),
];

#[test]
fn a_change_front_door_missing_an_operation_it_drives_or_the_terminal_approval_is_caught() {
    for (identity, operations) in CHANGE_DOORS {
        let body = served_text(identity);
        for needle in operations.iter().chain(&["baley scope approve"]) {
            assert!(body.contains(needle), "{identity} lacks {needle}");
        }
        assert!(!body.contains("scope-approve"), "{identity}");
    }
}

#[test]
fn a_change_front_door_asking_for_an_argument_its_operations_refuse_is_caught() {
    for (identity, _) in CHANGE_DOORS {
        assert!(
            !served_text(identity).contains(r#""instruction":"#),
            "{identity} asks for instruction"
        );
    }
}

#[test]
fn a_rounds_instruction_given_a_help_row_or_a_skill_stub_is_caught() {
    let Lookup::Served { entry, text } = lookup("bal-rounds") else {
        panic!("bal-rounds is served");
    };
    assert_eq!(entry.build, 4);
    assert_eq!(text.version, "2");
    assert!(
        !crate::help::table::COMMANDS
            .iter()
            .any(|command| command.name == "bal-rounds"),
        "bal-rounds is a help row"
    );
    assert!(
        !crate::host_artifacts::stubs::front_doors()
            .iter()
            .any(|(name, _)| *name == "bal-rounds"),
        "bal-rounds is a front door"
    );
}

#[test]
fn a_rounds_instruction_missing_its_read_its_answer_or_its_relay_rules_is_caught() {
    let body = served_text("bal-rounds");
    for needle in [
        r#""kind":"questions""#,
        "questions-answer",
        r#""act":"take""#,
        r#""act":"answer""#,
        r#""act":"defer""#,
        "recommended",
        "deferred",
        "closed",
        "baley questions answer",
    ] {
        assert!(body.contains(needle), "the rounds relay lacks {needle}");
    }
    let forbidding = body
        .split(". ")
        .find(|sentence| sentence.contains("drop"))
        .expect("a sentence forbids dropping");
    for word in ["answer", "drop", "merge", "reword", "reorder"] {
        assert!(forbidding.contains(word), "the forbidding lacks {word}");
    }
    assert!(!body.contains("approve"));
}

/// Names of Cadence's own files, commands and state that no ported analyzer
/// text may carry (ADR 0048).
const CADENCE_NAMES: &[&str] = &[
    ".planning",
    "CONTEXT.md",
    "ROADMAP.md",
    "planning.mjs",
    "skim.mjs",
    "excerpt",
    "recall",
    "AskUserQuestion",
    "/cad-",
];

#[test]
fn a_questions_text_missing_its_inputs_or_return_or_naming_cadence_state_is_caught() {
    let body = served_text("bal-analyzer-questions");
    for needle in [
        "`story`",
        "`description`",
        "`roadmap`",
        "`truths`",
        "`document`",
        "\"id\"",
        "\"question\"",
        "\"recommended\"",
        "\"depends_on\"",
        "`depends_on`",
        "`[]`",
        "Call no `baley_apply` operation",
    ] {
        assert!(body.contains(needle), "the questions text lacks {needle}");
    }
    for name in CADENCE_NAMES {
        assert!(!body.contains(name), "the questions text names {name}");
    }
}

#[test]
fn a_drafting_text_missing_the_form_refusals_or_human_verify_is_caught() {
    let body = served_text("bal-analyzer-drafting");
    for needle in [
        "`when`",
        "`for-any`",
        "`sees`",
        "`gets`",
        "`is refused`",
        "`truth-form`",
        "`truth-internal`",
        "`truth-prose-oracle`",
        "`human_verify`",
        "`questions`",
        "deferred question",
        "keeps that truth's id",
        "never on a recommendation the owner rejected",
        "Leave out `observable` and `fixed_oracle`",
    ] {
        assert!(body.contains(needle), "the drafting text lacks {needle}");
    }
    // The return's shape never carries the attestations the owner makes.
    assert!(!body.contains("\"observable\"") && !body.contains("\"fixed_oracle\""));
    for name in CADENCE_NAMES {
        assert!(!body.contains(name), "the drafting text names {name}");
    }
}

#[test]
fn an_analyzer_text_given_a_help_row_or_a_skill_stub_is_caught() {
    let doors = crate::host_artifacts::stubs::front_doors();
    for (identity, version) in [
        ("bal-analyzer-questions", "1"),
        ("bal-analyzer-drafting", "2"),
    ] {
        let Lookup::Served { entry, text } = lookup(identity) else {
            panic!("{identity} is served");
        };
        assert_eq!(entry.build, 4, "{identity}");
        assert_eq!(text.version, version, "{identity}");
        assert!(
            !crate::help::table::COMMANDS
                .iter()
                .any(|command| command.name == identity),
            "{identity} is a help row"
        );
        assert!(
            !doors.iter().any(|(name, _)| *name == identity),
            "{identity} is a front door"
        );
    }
}

#[test]
fn a_rounds_text_without_its_submit_step_or_editing_questions_is_caught() {
    let Lookup::Served { text, .. } = lookup("bal-rounds") else {
        panic!("bal-rounds is served");
    };
    assert_eq!(text.version, "2");
    let body = text.body;
    for needle in [
        "story-questions-submit",
        r#""work_order""#,
        r#""questions""#,
        "`drafting`",
        "a closed receipt with no drafting",
        "analyzer-order-pending",
    ] {
        assert!(body.contains(needle), "the rounds relay lacks {needle}");
    }
    let sending = body
        .split(". ")
        .find(|sentence| sentence.contains("exactly as returned"))
        .expect("a sentence sends the questions as returned");
    for word in ["edit", "drop", "merge", "reword", "reorder"] {
        assert!(sending.contains(word), "the sending lacks {word}");
    }
    let submit = body.find("story-questions-submit").unwrap();
    let read = body.find(r#""operation":"document""#).unwrap();
    assert!(submit < read, "the submit step comes after the round read");
}

#[test]
fn a_context_door_missing_an_operation_or_the_terminal_approval_is_caught() {
    let body = served_text("bal-context");
    for needle in [
        "story-refine",
        r#""instruction":"bal-context""#,
        "bal-rounds",
        "context-submit",
        r#"{"kind":"refinement""#,
        "work-order-fail",
        "work-order-retry",
        "analyzer-order-pending",
        "question-open",
        "deferring its open questions with a reason",
        "a closed receipt with no drafting",
        "observable",
        "fixed_oracle",
        "set only as the owner says",
        "human_verify",
        "baley scope preview",
        "baley story truths approve",
        "Nothing is approved over MCP",
    ] {
        assert!(
            body.contains(needle),
            "the context front door lacks {needle}"
        );
    }
    assert!(!body.contains("scope-approve") && !body.contains("truths-approve"));
}

#[test]
fn a_drafting_step_that_drops_truths_about_unbuilt_behaviour_is_caught() {
    // A story is refined before it is built, so the drop rule names a
    // contradiction, never a truth the code does not hold yet.
    for identity in ["bal-context", "bal-analyzer-drafting"] {
        let body = served_text(identity);
        assert!(
            body.contains("contradicts a fact the code holds or an answer the owner gave"),
            "{identity} drops on something other than a contradiction"
        );
        assert!(
            !body.contains("does not hold"),
            "{identity} drops a truth that does not hold"
        );
    }
    assert!(
        served_text("bal-context")
            .contains("A truth about behaviour the story has not built yet is kept."),
        "the context front door does not keep a truth about unbuilt behaviour"
    );
}

#[test]
fn a_context_door_keyed_by_phase_or_naming_cadence_state_is_caught() {
    let body = served_text("bal-context");
    assert!(
        body.contains("story's id"),
        "the context front door names no story id"
    );
    for name in CADENCE_NAMES.iter().chain(&["STATE", "cursor"]) {
        assert!(!body.contains(name), "the context front door names {name}");
    }
}

#[test]
fn a_default_test_style_missing_a_rule_of_its_design_row_is_caught() {
    // The five rules of design 0005 section 10's Default test style row.
    for rule in [
        "Test a unit through what it exposes",
        "Fake only the outside world",
        "Never start a program",
        "Skip trivial code",
        "Write the expected value by hand",
    ] {
        assert!(
            test_style::TEXT.contains(rule),
            "the default test style lacks {rule}"
        );
    }
}

/// The only numbers the planner text may hold: the two grammar limits
/// design 0005 states and the two phrases of its section 10 test rules that
/// count units, which the text carries word for word (D-18, D-19).
const PLANNER_NUMBERS: &[&str] = &[
    "at most 256 declarations",
    "at most 4,096 bytes",
    "two or more of the project's own units",
    "split a unit that touches two",
];

/// Count words a number handed to the planner would be spelled with.
const COUNT_WORDS: &[&str] = &[
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
    "thirty",
    "forty",
    "fifty",
    "sixty",
    "seventy",
    "eighty",
    "ninety",
    "hundred",
    "thousand",
    "dozen",
];

#[test]
fn a_number_handed_to_the_planner_is_caught() {
    let served = served_text("bal-phase-planner");
    let mut body = served.to_owned();
    for phrase in PLANNER_NUMBERS {
        body = body.replace(phrase, "");
    }
    if let Some(at) = body.find(|c: char| c.is_ascii_digit()) {
        let start = body.floor_char_boundary(at.saturating_sub(40));
        let end = body.ceil_char_boundary((at + 40).min(body.len()));
        panic!("the planner text holds a digit: {}", &body[start..end]);
    }
    for word in body.split(|c: char| !c.is_ascii_alphabetic()) {
        assert!(
            !COUNT_WORDS.contains(&word.to_ascii_lowercase().as_str()),
            "the planner text holds the count word {word}"
        );
    }
    for name in CADENCE_NAMES {
        assert!(!served.contains(name), "the planner text names {name}");
    }
    assert_eq!(
        lookup("bal-plan"),
        Lookup::Unavailable {
            identity: "bal-plan",
            build: 4
        }
    );
}

#[test]
fn a_planner_text_missing_its_grammar_or_a_refusal_is_caught() {
    let body = served_text("bal-phase-planner");
    // The plan's members, a task's, an item's and each spec's (Plan 1's
    // Context, D-01, D-12), the members Baley adds at approval (D-03), and
    // every refusal code of PLN-R11 to PLN-R13, PLN-R9 and PLN-R23.
    for name in [
        "phase",
        "stories",
        "files",
        "directories",
        "goal",
        "context",
        "notes",
        "tasks",
        "evidence_map",
        "questions",
        "id",
        "title",
        "action",
        "verify",
        "story",
        "truth",
        "truth_version",
        "kind",
        "spec",
        "reason",
        "artifact",
        "link",
        "check",
        "observation",
        "path",
        "record",
        "caller",
        "callee",
        "value",
        "what",
        "by",
        "command",
        "expected",
        "literal",
        "property",
        "test",
        "file",
        "function",
        "setup",
        "call",
        "level",
        "unit",
        "integration",
        "boundary",
        "fakes",
        "human_verify",
        "plan",
        "question_set",
        "size",
        "suite",
        "typed-content",
        "lease",
        "story-not-in-phase",
        "no-truths",
        "evidence-item-truth",
        "truth-version-mismatch",
        "evidence-item-conflict",
        "link-value-not-named",
        "uncovered-truth",
        "truth-without-check",
        "truth-without-observation",
        "truth-check-limit",
        "check-command",
        "check-expected",
        "check-test-file",
        "check-boundary",
        "over-capacity",
        "question-id",
        "question-blank",
        "question-dependency",
        "question-cycle",
    ] {
        assert!(
            body.contains(&format!("`{name}`")),
            "the planner text lacks `{name}`"
        );
    }
    for needle in [
        "propose a split rather than overfill",
        "put every decision the plan needs from the owner in `questions` with a recommended answer, write the plan on the recommended answers, and leave no decision in `notes` or task prose",
        "Proposed split:",
        "Call no `baley_apply` operation",
    ] {
        assert!(body.contains(needle), "the planner text lacks {needle}");
    }
}

#[test]
fn a_planner_text_without_the_test_rules_word_for_word_or_the_default_style_is_caught() {
    let body = served_text("bal-phase-planner");
    // Design 0005 section 10's test derivation rules, copied by hand
    // without the quote marks (PLN-R18, D-19).
    let rules = [
        "Treat a plan step as a container for work; separate its independently testable responsibilities instead of testing the whole step. For each responsibility, choose input classes, decision edges and failure responses justified by the requirement. Take expected values from that requirement; never invent behavior to make an answer available. Put each ambiguity the requirement leaves as a question in the plan's `questions` field, with a recommended answer, for the owner to decide.",
        "A generated unit test exercises one responsibility and one behavior, using at most one simulated external seam; split a unit that touches two. Run the real logic that owns the decision with supplied observations and independently justified expectations. In the one check's existing fields, connect the approved truth, production responsibility, inputs or seam, expected observable result and test. Name every other test in the task action with the meaningful defect it would catch. Stop when another case would distinguish no new required behavior or meaningful failure. Reuse adequate existing tests and relevant regressions; do not pursue test counts, a test per function, blanket permutations or coverage percentages. Use the managed project's approved language and test framework; Baley being written in Rust does not choose the project's language.",
        "Give every check a `level`. Write a unit check at one unit: `boundary` is the one unit the check exercises, named as a unit and not as a workflow, and `fakes` names every filesystem, process and clock seam that unit touches; do not leave it empty when the unit touches one. A check never starts a program to get its answer: not the project's binary, not git, not gpg, not a shell.",
        "When a truth's outcome depends on two or more of the project's own units working together, write its check as an integration check. Run those units together for real, and name them in `boundary`. Mock every outside boundary they reach, the database, the network, a third-party API, the clock, the filesystem or a host program, and name each in `fakes`. A mock sits behind an interface the project owns and serves the results and the failures the test needs. Never mock one of the project's own units inside an integration check. Where the project has no interface of its own at a boundary, say so in the plan and plan the seam first. Integration tests belong to the project's test command, so the suite runs them.",
        "Naming a fake is not permission to script one. A check builds the values it needs and asserts the rule over them. If the check has to supply the answer the unit is about to reach for, it measures nothing. When the unit stops in the middle of judging to ask git, the filesystem or the clock, that unit is not testable as written: say so in the plan and move the asking out, so the judging takes its facts as arguments. The small function that does the asking gets no check of its own.",
        "A generated test may rely only on the project's language toolchain and test libraries, including mocking libraries, from that language's package ecosystem. It must need no other language runtime, host-installed program, particular hardware or pre-existing machine state, and give the same result wherever the project builds. Test code starts no program. A test may create a fresh temporary directory as its one filesystem seam, keeps its reads and writes inside it, and makes no assertion depend on filesystem permissions, case sensitivity, symlink support or crash durability.",
        "When this phase first makes a running-program obligation runnable, add it to the map as a pending observation. Checks with mocked boundaries cannot establish real persistence, a real service, GUI interaction, performance or a live run of the assembled program, and passing them does not close that obligation. State what remains unverified.",
    ];
    for rule in rules {
        assert!(
            body.contains(rule),
            "the planner text lacks the rule beginning {}",
            &rule[..40]
        );
    }
    assert!(
        body.contains(test_style::TEXT),
        "the planner text lacks the default test style"
    );
}

#[test]
fn a_planner_text_given_a_help_row_or_a_skill_stub_is_caught() {
    let identity = "bal-phase-planner";
    let Lookup::Served { entry, text } = lookup(identity) else {
        panic!("{identity} is served");
    };
    assert_eq!(entry.build, 4);
    assert_eq!(text.version, "2");
    assert!(
        !crate::help::table::COMMANDS
            .iter()
            .any(|command| command.name == identity),
        "{identity} is a help row"
    );
    assert!(
        !crate::host_artifacts::stubs::front_doors()
            .iter()
            .any(|(name, _)| *name == identity),
        "{identity} is a front door"
    );
    assert!(!identity.starts_with("bal-planner-"));
}

#[test]
fn a_planner_input_list_missing_the_approved_plans_items_is_caught() {
    let Lookup::Served { text, .. } = lookup("bal-phase-planner") else {
        panic!("bal-phase-planner is served");
    };
    let start = text
        .body
        .find("<input>")
        .expect("the text has an input list");
    let end = text.body.find("</input>").expect("the input list ends");
    let list = &text.body[start..end];
    assert!(
        list.contains("the evidence items of the phase's approved plans"),
        "the input list lacks the approved plans' items: {list}"
    );
    assert!(
        list.contains("the phase's number, goal and detail"),
        "{list}"
    );
    assert_eq!(text.version, "2");
}

#[test]
fn a_planner_text_naming_cadence_files_or_markers_is_caught() {
    let body = served_text("bal-phase-planner");
    for name in [
        "PLAN.md",
        "PLAN-",
        "frontmatter",
        "STATE",
        "cursor",
        "## ",
        "Task ceiling",
        "lease-check",
        "undeclared-files",
        "max_plan_tasks",
    ] {
        assert!(!body.contains(name), "the planner text names {name}");
    }
}

#[test]
fn a_plan_door_missing_a_step_it_owns_is_caught() {
    // The steps D-17 keeps: the planner order, its launch, failure and
    // retry, the submit, the rounds, the split and the terminal approval.
    for needle in [
        "phase-plan",
        r#""instruction":"bal-plan""#,
        "launch",
        "prompt",
        "work-order-fail",
        "work-order-retry",
        "plan-submit",
        "bal-rounds",
        "Proposed split:",
        "over-capacity",
        "question-open",
        "baley scope preview",
        "baley plan approve",
        "Nothing is approved over MCP",
    ] {
        assert!(
            plan::TEXT.contains(needle),
            "the plan front door lacks {needle}"
        );
    }
}

#[test]
fn a_plan_door_naming_cadence_state_or_a_later_tasks_step_is_caught() {
    for name in CADENCE_NAMES.iter().chain(&[
        "STATE",
        "cursor",
        "PLAN.md",
        "checker",
        "plan_check",
        "--gaps",
        "--inline",
        "--skip-check",
        "seed-reqs",
        "cite-count",
        "lease-check",
        "max_plan_tasks",
    ]) {
        assert!(
            !plan::TEXT.contains(name),
            "the plan front door names {name}"
        );
    }
}
