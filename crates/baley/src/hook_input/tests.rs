//! Hook input classified over plain bytes. No test touches a filesystem or
//! starts a program.

use super::{CommandTool, Envelope, HookInput, MAX_INPUT_BYTES, classify};

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
