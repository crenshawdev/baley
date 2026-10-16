//! `CLAUDE_PROJECT_DIR` as the environment gives it, judged by the server and the hook.
#![no_main]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use baley::mcp::context::project_context;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let value = OsStr::from_bytes(data);
    let _ = project_context(Some(value), true);
    let _ = project_context(Some(value), false);
});
