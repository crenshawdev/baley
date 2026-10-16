//! The MCP server's line framing over stdio, fed one byte at a time as the transport does.
#![no_main]

use baley::mcp::frame::Decoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut decoder = Decoder::new();
    for byte in data {
        let _ = decoder.push(*byte);
    }
});
