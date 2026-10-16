//! The update manifest downloaded from `updates.source`.
#![no_main]

use baley::update::manifest::parse_manifest;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = parse_manifest(data);
});
