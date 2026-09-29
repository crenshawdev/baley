//! The `[project]` table of the project file: the project's id and name
//! (design 0003, CFG-R3, ADR 0004).

/// Whether `id` is a project id: a UUID version 4 written as 36 bytes of
/// lower-case hex in 8-4-4-4-12 groups, with version nibble `4` and variant
/// nibble `8`, `9`, `a` or `b`. Nothing else is accepted, so every clone
/// reads the one id byte for byte.
pub fn is_project_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => matches!(byte, b'0'..=b'9' | b'a'..=b'f'),
        })
        && bytes[14] == b'4'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
}
