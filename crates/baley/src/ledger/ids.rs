//! Request identities created at the edge.
use baley_store::RequestId;
use std::{
    fs::File,
    io::{self, Read},
};

/// Reads fresh entropy for one request identity.
pub(super) fn fresh_request_id() -> io::Result<RequestId> {
    let mut bytes = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(RequestId(uuid_v4_text(bytes)))
}
/// Sets the UUID version and variant and formats its bytes.
pub(super) fn uuid_v4_text(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
