use super::{Tally, count_reads, report_lines, resolve, select_workers};
use crate::read::model::DocumentIdentity;
use serde_json::json;
use std::collections::BTreeMap;
use std::num::NonZeroU32;

#[test]
fn transcript_reads_are_counted_by_kind() {
    let claude = vec![
        json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": "r1", "name": "Read", "input": {"file_path": "/p/src/a.rs"}},
            {"type": "tool_use", "id": "r2", "name": "Read", "input": {"file_path": "/p/src/a.rs", "offset": 10, "limit": 20}},
            {"type": "tool_use", "id": "g1", "name": "Grep", "input": {"pattern": "needle"}},
            {"type": "tool_use", "id": "b1", "name": "Bash", "input": {"command": "cat src/a.rs"}},
            {"type": "tool_use", "id": "b2", "name": "Bash", "input": {"command": "make"}},
            {"type": "tool_use", "id": "q1", "name": "mcp__baley__baley_query", "input": {"operation": "search"}},
            {"type": "tool_use", "id": "q2", "name": "mcp__baley__baley_query", "input": {"operation": "read"}},
            {"type": "tool_use", "id": "q3", "name": "mcp__baley__baley_query", "input": {"operation": "document"}}
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
        count_reads(&claude, &read_lines).unwrap(),
        BTreeMap::from([
            (
                "Read whole".into(),
                Tally {
                    calls: 1,
                    bytes: 10
                }
            ),
            (
                "Read ranged".into(),
                Tally {
                    calls: 1,
                    bytes: 20
                }
            ),
            ("Grep".into(), Tally { calls: 1, bytes: 8 }),
            (
                "shell read".into(),
                Tally {
                    calls: 1,
                    bytes: 30
                }
            ),
            ("unclassified".into(), Tally { calls: 1, bytes: 5 }),
            (
                "baley_query search".into(),
                Tally {
                    calls: 1,
                    bytes: 40
                }
            ),
            (
                "baley_query read".into(),
                Tally {
                    calls: 1,
                    bytes: 50
                }
            ),
            (
                "baley_query document".into(),
                Tally {
                    calls: 1,
                    bytes: 60
                }
            ),
        ])
    );
}

#[test]
fn every_agent_call_in_the_episode_selects_its_worker() {
    let main = vec![json!({"message": {"content": [
        {"type": "tool_use", "id": "a1", "name": "Agent", "input": {"subagent_type": "general-purpose"}},
        {"type": "tool_use", "id": "a2", "name": "Agent", "input": {"subagent_type": "bal-planner"}}
    ]}})];
    let workers = vec![
        (
            "a1".into(),
            vec![json!({"message": {"content": [
                {"type": "tool_use", "id": "a3", "name": "Agent", "input": {"subagent_type": "general-purpose"}}
            ]}})],
        ),
        ("a2".into(), vec![]),
        ("a3".into(), vec![]),
        ("zz".into(), vec![]),
    ];
    assert_eq!(select_workers(&main, &workers), vec![0, 1, 2]);
}

#[test]
fn an_episode_without_agent_calls_selects_no_worker() {
    let main = vec![json!({"message": {"content": [{"type": "text", "text": "planning"}]}})];
    let workers = vec![("a9".into(), vec![])];
    assert_eq!(select_workers(&main, &workers), Vec::<usize>::new());
}

#[test]
fn the_report_lists_each_kind_with_its_calls_and_bytes() {
    let counts = BTreeMap::from([
        (
            "Grep".into(),
            Tally {
                calls: 2,
                bytes: 30,
            },
        ),
        (
            "baley_query read".into(),
            Tally {
                calls: 1,
                bytes: 50,
            },
        ),
    ]);
    assert_eq!(
        report_lines(&counts),
        "reads[Grep]: 2 calls, 30 bytes\nreads[baley_query read]: 1 calls, 50 bytes\n"
    );
}

#[test]
fn a_planner_round_session_id_that_is_not_a_uuid_is_refused_as_an_identity() {
    // The planning root's parent was never created, so a check that let the
    // id through would stop at the unavailable-project refusal instead.
    let dir = tempfile::tempdir().unwrap();
    let planning_root = dir.path().join("missing").join(".planning");
    let identity = DocumentIdentity::PlannerRound {
        phase: NonZeroU32::new(2).unwrap(),
        session_id: "01a0cf16".into(),
        first_turn: "01a0cf16-cbd7-7910-be17-2003ce72549d".into(),
        last_turn: "01a0cef8-bb58-7f22-8729-5c0e1e82b0f9".into(),
    };
    assert_eq!(
        resolve(&planning_root, &identity).err().unwrap()["code"],
        "document-identity"
    );
}
