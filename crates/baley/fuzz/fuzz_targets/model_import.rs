//! A provider's model list piped into `baley models import`, with its trailer.
#![no_main]

use baley::detection::parse_fetched;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = parse_fetched(data);
});
