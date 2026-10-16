//! The JSON Claude Code sends the guard hook on stdin before a tool call.
#![no_main]

use baley::hook_input::classify;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = classify(data);
});
