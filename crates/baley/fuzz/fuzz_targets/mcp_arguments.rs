//! A tool call's arguments and JSON-RPC id, judged by each operation the server serves.
#![no_main]

use baley::mcp::context::judge_id;
use baley::mcp::{capture, document, operations, scope};
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    // The frame decoder hands on only valid JSON.
    let Ok(arguments) = serde_json::from_slice::<Value>(data) else {
        return;
    };
    let _ = judge_id(&arguments);
    let _ = operations::spelling(arguments.as_object());
    let _ = operations::help_answer(&arguments);
    let _ = operations::schema_answer(&arguments);
    let _ = operations::instruction_answer(&arguments);
    let _ = document::judge_arguments(&arguments);
    let _ = capture::judge_arguments(&arguments);
    let _ = scope::judge_arguments(&arguments);
});
