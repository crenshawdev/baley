//! The guard's scan of a Bash or Monitor command, which the agent writes.
#![no_main]

use baley_core::guard::{git_command, git_verb};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The hook input is JSON, so a command always arrives as UTF-8.
    let Ok(command) = std::str::from_utf8(data) else {
        return;
    };
    let _ = git_verb(command);
    let _ = git_command(command);
});
