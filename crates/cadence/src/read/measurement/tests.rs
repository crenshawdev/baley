use super::{Host, Tally, count_reads};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn transcript_reads_are_counted_by_kind() {
    let claude = vec![
        json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": "r1", "name": "Read", "input": {"file_path": "/p/src/a.rs"}},
            {"type": "tool_use", "id": "r2", "name": "Read", "input": {"file_path": "/p/src/a.rs", "offset": 10, "limit": 20}},
            {"type": "tool_use", "id": "g1", "name": "Grep", "input": {"pattern": "needle"}},
            {"type": "tool_use", "id": "b1", "name": "Bash", "input": {"command": "cat src/a.rs"}},
            {"type": "tool_use", "id": "b2", "name": "Bash", "input": {"command": "make"}},
            {"type": "tool_use", "id": "q1", "name": "mcp__cadence__cadence_query", "input": {"operation": "search"}},
            {"type": "tool_use", "id": "q2", "name": "mcp__cadence__cadence_query", "input": {"operation": "read"}},
            {"type": "tool_use", "id": "q3", "name": "mcp__cadence__cadence_query", "input": {"operation": "document"}}
        ]}}),
        json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "r1", "content": "a".repeat(10)},
            {"type": "tool_result", "tool_use_id": "r2", "content": "b".repeat(20)},
            {"type": "tool_result", "tool_use_id": "g1", "content": "c".repeat(8)},
            {"type": "tool_result", "tool_use_id": "b1", "content": "d".repeat(30)},
            {"type": "tool_result", "tool_use_id": "b2", "content": "e".repeat(5)},
            {"type": "tool_result", "tool_use_id": "q1", "content": [{"type": "text", "text": "f".repeat(40)}]},
            {"type": "tool_result", "tool_use_id": "q2", "content": [{"type": "text", "text": "g".repeat(50)}]},
            {"type": "tool_result", "tool_use_id": "q3", "content": [{"type": "text", "text": "h".repeat(60)}]}
        ]}}),
    ];
    let read_lines = BTreeMap::from([("/p/src/a.rs".into(), 100)]);
    assert_eq!(
        count_reads(Host::Claude, &claude, &read_lines).unwrap(),
        BTreeMap::from([
            ("Read whole".into(), Tally { calls: 1, bytes: 10 }),
            ("Read ranged".into(), Tally { calls: 1, bytes: 20 }),
            ("Grep".into(), Tally { calls: 1, bytes: 8 }),
            ("shell read".into(), Tally { calls: 1, bytes: 30 }),
            ("unclassified".into(), Tally { calls: 1, bytes: 5 }),
            ("cadence_query search".into(), Tally { calls: 1, bytes: 40 }),
            ("cadence_query read".into(), Tally { calls: 1, bytes: 50 }),
            ("cadence_query document".into(), Tally { calls: 1, bytes: 60 }),
        ])
    );

    let codex = vec![
        json!({"type": "event_msg", "payload": {"type": "item_completed", "item": {
            "type": "McpToolCall", "server": "cadence", "tool": "cadence_query", "arguments": {"operation": "search"},
            "result": {"content": [{"type": "text", "text": "s".repeat(100)}], "structuredContent": {"k": "v"}}
        }}}),
        json!({"type": "event_msg", "payload": {"type": "item_completed", "item": {
            "type": "McpToolCall", "server": "cadence", "tool": "cadence_query", "arguments": {"operation": "read"},
            "result": {"content": [{"type": "text", "text": "r".repeat(200)}], "structuredContent": {"k": "v"}}
        }}}),
        json!({"type": "event_msg", "payload": {"type": "item_completed", "item": {
            "type": "McpToolCall", "server": "cadence", "tool": "cadence_apply", "arguments": {"operation": "plan-submit"},
            "result": {"content": [{"type": "text", "text": "a".repeat(70)}]}
        }}}),
        json!({"type": "event_msg", "payload": {"type": "item_completed", "item": {
            "type": "CommandExecution", "parsed_cmd": [{"type": "read"}], "aggregated_output": "b".repeat(300)
        }}}),
        json!({"type": "event_msg", "payload": {"type": "item_completed", "item": {
            "type": "CommandExecution", "parsed_cmd": [{"type": "search"}], "aggregated_output": "c".repeat(400)
        }}}),
        json!({"type": "event_msg", "payload": {"type": "item_completed", "item": {
            "type": "CommandExecution", "parsed_cmd": [{"type": "unknown", "cmd": "cargo nextest run"}], "aggregated_output": "d".repeat(7)
        }}}),
        json!({"type": "event_msg", "payload": {"type": "item_completed", "item": {"type": "AgentMessage"}}}),
        json!({"type": "event_msg", "payload": {"type": "item_completed", "item": {"type": "Extension"}}}),
        json!({"type": "event_msg", "payload": {"type": "item_completed"}}),
        json!({"type": "event_msg", "payload": {"type": "token_count"}}),
        json!({"type": "response_item", "payload": {"type": "message"}}),
    ];
    assert_eq!(
        count_reads(Host::Codex, &codex, &BTreeMap::new()).unwrap(),
        BTreeMap::from([
            ("cadence_query search".into(), Tally { calls: 1, bytes: 100 }),
            ("cadence_query read".into(), Tally { calls: 1, bytes: 200 }),
            ("shell read".into(), Tally { calls: 2, bytes: 700 }),
            ("unclassified".into(), Tally { calls: 3, bytes: 7 }),
        ])
    );
}
