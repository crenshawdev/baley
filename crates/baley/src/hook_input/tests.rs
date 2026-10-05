//! Hook input classified over plain bytes. No test touches a filesystem or
//! starts a program.

use super::{CommandTool, Envelope, HookInput, MAX_INPUT_BYTES, PathTarget, PathTool, classify};

fn bytes(tool: &str, tool_input: &str) -> Vec<u8> {
    format!(
        r#"{{"hook_event_name":"PreToolUse","tool_name":"{tool}","cwd":"/p","session_id":"s1","tool_use_id":"t1","tool_input":{tool_input}}}"#
    )
    .into_bytes()
}

fn envelope() -> Envelope {
    Envelope {
        cwd: "/p".into(),
        session_id: Some("s1".into()),
        tool_use_id: Some("t1".into()),
    }
}

fn command(tool: CommandTool, text: &str) -> HookInput {
    HookInput::Command {
        envelope: envelope(),
        tool,
        text: text.into(),
    }
}

fn path(tool: PathTool, path: Option<&str>, pattern: Option<&str>) -> HookInput {
    HookInput::Path {
        envelope: envelope(),
        target: PathTarget {
            tool,
            path: path.map(Into::into),
            pattern: pattern.map(Into::into),
        },
    }
}

fn is_deny(input: &HookInput) -> bool {
    matches!(input, HookInput::Deny(_))
}

#[test]
fn a_bash_and_a_monitor_command_give_the_same_text_so_monitor_commands_are_not_skipped() {
    let text = "git push origin main";
    let input = format!(r#"{{"command":"{text}","description":"d"}}"#);
    assert_eq!(
        classify(&bytes("Bash", &input)),
        command(CommandTool::Bash, text)
    );
    assert_eq!(
        classify(&bytes("Monitor", &input)),
        command(CommandTool::Monitor, text)
    );
}

#[test]
fn a_monitor_input_with_no_command_is_a_watch_and_not_a_shell_command() {
    assert_eq!(
        classify(&bytes(
            "Monitor",
            r#"{"ws_url":"wss://x/y","persistent":true}"#
        )),
        HookInput::Watch(envelope())
    );
}

#[test]
fn a_monitor_command_of_null_or_a_wrong_type_is_not_read_as_a_missing_command() {
    for input in [r#"{"command":null}"#, r#"{"command":7}"#] {
        assert_eq!(
            classify(&bytes("Monitor", input)),
            HookInput::NoAnswer,
            "{input}"
        );
    }
}

#[test]
fn powershell_with_git_push_is_the_powershell_class_and_never_a_command() {
    assert_eq!(
        classify(&bytes(
            "PowerShell",
            r#"{"command":"git push origin main"}"#
        )),
        HookInput::PowerShell(envelope())
    );
}

#[test]
fn powershell_with_an_unfamiliar_tool_input_is_still_powershell_so_no_field_is_required() {
    for input in [r#"{"script":"ls"}"#, r#""just text""#, r#"[1]"#, "null"] {
        assert_eq!(
            classify(&bytes("PowerShell", input)),
            HookInput::PowerShell(envelope()),
            "{input}"
        );
    }
    let without_input =
        br#"{"tool_name":"PowerShell","cwd":"/p","session_id":"s1","tool_use_id":"t1"}"#;
    assert_eq!(classify(without_input), HookInput::PowerShell(envelope()));
}

#[test]
fn a_powershell_input_that_cannot_be_read_gets_no_answer_and_is_not_denied() {
    let wrong_event = br#"{"hook_event_name":"PostToolUse","tool_name":"PowerShell","cwd":"/p"}"#;
    assert_eq!(classify(wrong_event), HookInput::NoAnswer);
    assert_eq!(
        classify(br#"{"tool_name":"PowerShell","cwd":3}"#),
        HookInput::NoAnswer
    );
    assert_eq!(
        classify(br#"{"tool_name":"PowerShell","cwd":"/p"} x"#),
        HookInput::NoAnswer
    );
}

#[test]
fn a_malformed_wrong_typed_or_oversized_bash_input_gets_no_answer_so_a_declined_command_is_not_denied()
 {
    let oversized = {
        let mut input = br#"{"tool_name":"Bash","cwd":"/p","tool_input":{"command":""#.to_vec();
        input.resize(MAX_INPUT_BYTES as usize + 1, b'x');
        input
    };
    let cases: Vec<Vec<u8>> = vec![
        bytes("Bash", r#"{"command":null}"#),
        bytes("Bash", r#"{"command":["a"]}"#),
        bytes("Bash", r#"{"command":"a","command":"b"}"#),
        bytes("Bash", "{}"),
        br#"{"tool_name":"Bash","cwd":"/p"}"#.to_vec(),
        br#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#.to_vec(),
        br#"{"tool_name":"Bash","cwd":7,"tool_input":{"command":"ls"}}"#.to_vec(),
        br#"{"tool_name":"Bash","cwd":"/p","tool_input":{"command":"ls"}} {}"#.to_vec(),
        br#"["Bash"]"#.to_vec(),
        br#"{"tool_name":"Bash", oops"#.to_vec(),
        b"not json".to_vec(),
        [br#"{"tool_name":"Bash","#.as_slice(), &[0xff]].concat(),
        br#"{"hook_event_name":"PostToolUse","tool_name":"Bash","cwd":"/p","tool_input":{"command":"ls"}}"#.to_vec(),
        oversized,
    ];
    for case in cases {
        assert_eq!(
            classify(&case),
            HookInput::NoAnswer,
            "{}",
            String::from_utf8_lossy(&case[..case.len().min(200)])
        );
    }
}

#[test]
fn a_missing_or_empty_tool_use_id_gives_no_call_id_so_none_is_invented() {
    let input = r#"{"command":"ls"}"#;
    let without = format!(r#"{{"tool_name":"Bash","cwd":"/p","tool_input":{input}}}"#);
    let empty =
        format!(r#"{{"tool_name":"Bash","cwd":"/p","tool_use_id":"","tool_input":{input}}}"#);
    for case in [without, empty] {
        let HookInput::Command { envelope, .. } = classify(case.as_bytes()) else {
            panic!("{case}");
        };
        assert_eq!(envelope.tool_use_id, None, "{case}");
        assert_eq!(envelope.session_id, None, "{case}");
    }
    let HookInput::Command { envelope, .. } = classify(&bytes("Bash", input)) else {
        panic!("a given id is carried");
    };
    assert_eq!(envelope.tool_use_id.as_deref(), Some("t1"));
    assert_eq!(envelope.session_id.as_deref(), Some("s1"));
}

#[test]
fn an_unknown_tool_gets_no_answer() {
    assert_eq!(
        classify(&bytes("WebFetch", r#"{"url":"x"}"#)),
        HookInput::NoAnswer
    );
    assert_eq!(
        classify(br#"{"tool_name":7,"cwd":"/p"}"#),
        HookInput::NoAnswer
    );
    assert_eq!(classify(br#"{"cwd":"/p"}"#), HookInput::NoAnswer);
}

#[test]
fn notebook_edit_gives_its_notebook_path_as_the_target() {
    assert_eq!(
        classify(&bytes(
            "NotebookEdit",
            r#"{"notebook_path":"/p/a.ipynb","new_source":"x"}"#
        )),
        path(PathTool::NotebookEdit, Some("/p/a.ipynb"), None)
    );
}

#[test]
fn read_write_and_edit_give_their_file_path() {
    for (name, tool) in [
        ("Read", PathTool::Read),
        ("Write", PathTool::Write),
        ("Edit", PathTool::Edit),
    ] {
        assert_eq!(
            classify(&bytes(name, r#"{"file_path":"/p/a.rs","content":"x"}"#)),
            path(tool, Some("/p/a.rs"), None),
            "{name}"
        );
    }
}

#[test]
fn grep_and_glob_without_a_path_carry_no_path() {
    assert_eq!(
        classify(&bytes(
            "Grep",
            r#"{"pattern":"todo","output_mode":"content"}"#
        )),
        path(PathTool::Grep, None, None)
    );
    assert_eq!(
        classify(&bytes("Glob", r#"{"pattern":"**/*.rs"}"#)),
        path(PathTool::Glob, None, Some("**/*.rs"))
    );
}

#[test]
fn a_glob_pattern_and_a_grep_glob_are_carried_and_a_grep_search_pattern_is_not() {
    assert_eq!(
        classify(&bytes("Glob", r#"{"pattern":"src/*.rs","path":"/p"}"#)),
        path(PathTool::Glob, Some("/p"), Some("src/*.rs"))
    );
    assert_eq!(
        classify(&bytes(
            "Grep",
            r#"{"pattern":"fn main","glob":"*.rs","path":"/p/src"}"#
        )),
        path(PathTool::Grep, Some("/p/src"), Some("*.rs"))
    );
}

#[test]
fn malformed_read_and_glob_input_is_denied_so_their_silence_is_gone() {
    assert!(is_deny(&classify(
        br#"{"tool_name":"Read","cwd":"/p","tool_input":{"file_path":null}}"#
    )));
    assert!(is_deny(&classify(br#"{"tool_name":"Glob", oops"#)));
}

#[test]
fn a_path_tool_deny_names_the_tool() {
    let HookInput::Deny(reason) = classify(br#"{"tool_name":"Glob", oops"#) else {
        panic!("denied");
    };
    assert!(reason.starts_with("Glob "), "{reason}");
}

#[test]
fn a_path_tool_input_that_is_not_one_readable_event_is_denied() {
    for case in [
        r#"{"tool_name":"Write","cwd":"/p","tool_input":{"file_path":null}}"#,
        r#"{"tool_name":"Write","cwd":1,"tool_input":{"file_path":"x"}}"#,
        r#"{"tool_name":"Write","tool_input":{"file_path":"x"}}"#,
        r#"{"tool_name":"Edit","cwd":"/","tool_input":{}}"#,
        r#"{"tool_name":"Edit","cwd":"/"}"#,
        r#"{"tool_name":"Edit","cwd":"/","tool_input":"x"}"#,
        r#"{"tool_name":"Grep","cwd":"/p","tool_input":{"path":3}}"#,
        r#"{"tool_name":"Grep","cwd":"/p","tool_input":{"glob":null}}"#,
        r#"{"tool_name":"NotebookEdit","cwd":"/p","tool_input":{"file_path":"/p/a.ipynb"}}"#,
        r#"{"tool_name":"Read","cwd":"/p","tool_input":{"file_path":"x"}} {}"#,
        r#"{"tool_name":"Read","cwd":"/p","cwd":"/q","tool_input":{"file_path":"x"}}"#,
        r#"{"tool_name":"Read","tool_name":"Read","cwd":"/p","tool_input":{"file_path":"x"}}"#,
        r#"{"hook_event_name":"PostToolUse","tool_name":"Read","cwd":"/p","tool_input":{"file_path":"x"}}"#,
    ] {
        assert!(is_deny(&classify(case.as_bytes())), "{case}");
    }
    let invalid_utf8 = [br#"{"tool_name":"Write","#.as_slice(), &[0xff]].concat();
    assert!(is_deny(&classify(&invalid_utf8)));
}

#[test]
fn a_write_with_file_path_given_twice_is_denied() {
    assert!(is_deny(&classify(&bytes(
        "Write",
        r#"{"file_path":"/a","file_path":"/b"}"#
    ))));
}

#[test]
fn a_glob_with_no_pattern_is_denied() {
    assert!(is_deny(&classify(&bytes("Glob", r#"{"path":"/p"}"#))));
}

#[test]
fn a_malformed_input_names_a_path_tool_through_any_whitespace() {
    assert!(is_deny(&classify(b"{ \"tool_name\" :\n \"Grep\" , oops")));
    assert!(is_deny(&classify(
        b"{ \"tool_name\" :\n \"NotebookEdit\" , oops"
    )));
}

#[test]
fn a_malformed_input_for_a_tool_name_that_only_starts_like_a_path_tool_gets_no_answer() {
    for case in [
        br#"{"tool_name":"Reader", oops"#.as_slice(),
        br#"{"tool_name":"Writer", oops"#,
        br#"{"tool_name":"Reader","cwd":"/p","tool_input":{}}"#,
    ] {
        assert_eq!(
            classify(case),
            HookInput::NoAnswer,
            "{}",
            String::from_utf8_lossy(case)
        );
    }
}

#[test]
fn a_grep_input_is_denied_one_byte_past_the_bound_and_classified_at_it() {
    let sized = |fill: usize| {
        let mut input = bytes("Grep", r#"{"pattern":"x"}"#);
        input.resize(input.len() + fill, b' ');
        input
    };
    let fill = MAX_INPUT_BYTES as usize - sized(0).len();
    let at_bound = sized(fill);
    assert_eq!(at_bound.len() as u64, MAX_INPUT_BYTES);
    assert_eq!(classify(&at_bound), path(PathTool::Grep, None, None));
    assert!(is_deny(&classify(&sized(fill + 1))));
}

#[test]
fn an_oversized_input_for_a_command_tool_is_not_denied() {
    let mut input = bytes("Monitor", r#"{"command":"ls"}"#);
    input.resize(MAX_INPUT_BYTES as usize + 1, b' ');
    assert_eq!(classify(&input), HookInput::NoAnswer);
}
