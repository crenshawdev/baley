//! Canonical JSON: RFC 8785 (the JSON Canonicalization Scheme) over the
//! ledger's number domain.
//!
//! The same event must hash the same way on every platform (design 0001,
//! Events), so envelopes and payloads are hashed from these bytes and never
//! from whatever a serializer happened to emit. The RFC serializes numbers as
//! IEEE doubles; Baley records only counts, sequences and sizes, so a number
//! is an integer within ±(2^53 − 1) or it is refused at the door (Build 1,
//! ruling 2). Inside that domain the RFC's number formatting is the plain
//! decimal digits, so no double ever enters the picture.

use std::fmt;

use serde_json::Value;

/// The largest integer the RFC can carry without loss: 2^53 − 1.
pub const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

/// Why a value has no canonical form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalError {
    /// A number that is not an integer. `path` locates it, `number` is its
    /// source text.
    NotAnInteger { path: String, number: String },
    /// An integer outside ±(2^53 − 1).
    OutOfRange { path: String, number: String },
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnInteger { path, number } => {
                write!(f, "{number} at {path} is not an integer")
            }
            Self::OutOfRange { path, number } => {
                write!(f, "{number} at {path} is outside ±(2^53 - 1)")
            }
        }
    }
}

impl std::error::Error for CanonicalError {}

/// The canonical bytes of `value`, or the first reason it has none.
///
/// Objects are written with their keys sorted by UTF-16 code units, arrays in
/// order, strings with the RFC's escapes and nothing else, integers as
/// decimal digits, and no whitespace.
pub fn canonical_json(value: &Value) -> Result<Vec<u8>, CanonicalError> {
    let mut out = Vec::new();
    let mut path = String::from("$");
    write_value(value, &mut out, &mut path)?;
    Ok(out)
}

fn write_value(value: &Value, out: &mut Vec<u8>, path: &mut String) -> Result<(), CanonicalError> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => {
            // With `arbitrary_precision` the number keeps its source text,
            // so `-0` reads as the integer 0 and `1.0` or `1e2` do not read
            // as integers at all.
            let text = number.to_string();
            let digits = text.strip_prefix('-').unwrap_or(&text);
            let integer_token = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
            let in_range = number
                .as_i64()
                .filter(|integer| (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(integer));
            let Some(integer) = in_range else {
                return Err(if integer_token {
                    CanonicalError::OutOfRange {
                        path: path.clone(),
                        number: text,
                    }
                } else {
                    CanonicalError::NotAnInteger {
                        path: path.clone(),
                        number: text,
                    }
                });
            };
            out.extend_from_slice(integer.to_string().as_bytes());
        }
        Value::String(string) => write_string(string, out),
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                let mark = path.len();
                path.push_str(&format!("[{index}]"));
                write_value(item, out, path)?;
                path.truncate(mark);
            }
            out.push(b']');
        }
        Value::Object(members) => {
            // Sorted by the UTF-16 code units of the key, as the RFC says:
            // a byte sort of the UTF-8 puts U+E000 before U+10000, UTF-16
            // puts it after.
            let mut entries: Vec<(&String, &Value)> = members.iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
            out.push(b'{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_string(key, out);
                out.push(b':');
                let mark = path.len();
                path.push('.');
                path.push_str(key);
                write_value(item, out, path)?;
                path.truncate(mark);
            }
            out.push(b'}');
        }
    }
    Ok(())
}

/// RFC 8785 section 3.2.2.2: the two-character escapes for the characters
/// that have them, `\u00xx` in lower case for the other controls, and every
/// other character as itself.
fn write_string(string: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for character in string.chars() {
        match character {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{8}' => out.extend_from_slice(b"\\b"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\u{c}' => out.extend_from_slice(b"\\f"),
            '\r' => out.extend_from_slice(b"\\r"),
            control if (control as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", control as u32).as_bytes());
            }
            other => {
                let mut buffer = [0u8; 4];
                out.extend_from_slice(other.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn canonical(value: Value) -> String {
        String::from_utf8(canonical_json(&value).expect("canonical")).expect("utf-8")
    }

    // RFC 8785 section 3.2.2, the array example: the object's keys come out
    // sorted and the whitespace goes. Catches a serializer that keeps
    // insertion order or pretty-prints.
    #[test]
    fn rfc_array_example_sorts_keys_and_drops_whitespace() {
        let value: Value =
            serde_json::from_str(r#"[56, {"d": true, "10": null, "1": [ ]}]"#).expect("parse");
        assert_eq!(canonical(value), r#"[56,{"1":[],"10":null,"d":true}]"#);
    }

    // RFC 8785 section 3.2.3, the string and literal members of the example
    // (its numbers are doubles, outside this domain). Expected bytes are the
    // RFC's own output for those members. Catches escaping of `/`, of
    // non-ASCII, upper-case `\u` hex, and a missing `\n` short form.
    #[test]
    fn rfc_string_example_escapes_as_the_rfc_says() {
        let value: Value = serde_json::from_str(
            "{\"string\": \"\\u20ac$\\u000F\\u000aA'\\u0042\\u0022\\u005c\\\\\\\"\\/\", \"literals\": [null, true, false]}",
        )
        .expect("parse");
        assert_eq!(
            canonical(value),
            "{\"literals\":[null,true,false],\"string\":\"\u{20ac}$\\u000f\\nA'B\\\"\\\\\\\\\\\"/\"}"
        );
    }

    // Nested objects sort at every level, not only the top. Catches a sort
    // applied once.
    #[test]
    fn nested_objects_sort_at_every_level() {
        let value = json!({"z": {"b": 1, "a": {"y": 2, "x": 3}}, "a": [{"k": 1, "j": 2}]});
        assert_eq!(
            canonical(value),
            r#"{"a":[{"j":2,"k":1}],"z":{"a":{"x":3,"y":2},"b":1}}"#
        );
    }

    // U+E000 is EE 80 80 in UTF-8 and one code unit E000 in UTF-16; U+10000
    // is F0 90 80 80 in UTF-8 and the pair D800 DC00 in UTF-16. Bytes put
    // U+E000 first, code units put U+10000 first; the RFC wants code units.
    // Catches a byte-order key sort.
    #[test]
    fn keys_sort_by_utf16_code_units_not_utf8_bytes() {
        let value = json!({"\u{E000}": 1, "\u{10000}": 2});
        assert_eq!(canonical(value), "{\"\u{10000}\":2,\"\u{E000}\":1}");
    }

    // Every control character below U+0020 without a short form is written
    // as lower-case `\u00xx`; U+007F and above are written as themselves.
    // Catches escaping U+007F or U+2028, and upper-case hex.
    #[test]
    fn controls_without_short_forms_use_lowercase_u_escapes() {
        let value = json!("\u{1}\u{1f}\u{7f}\u{2028}");
        assert_eq!(canonical(value), "\"\\u0001\\u001f\u{7f}\u{2028}\"");
    }

    // 2^53 − 1 is the largest integer the RFC carries exactly, in both signs.
    // Catches a bound placed one off.
    #[test]
    fn integers_within_the_safe_range_are_written_as_digits() {
        let value = json!([9007199254740991_i64, -9007199254740991_i64, 0]);
        assert_eq!(canonical(value), "[9007199254740991,-9007199254740991,0]");
    }

    // 2^53 itself is refused, named by its path. Catches a bound that lets
    // the first inexact integer through.
    #[test]
    fn two_to_the_fifty_three_is_refused() {
        let value = json!({"count": 9007199254740992_i64});
        assert_eq!(
            canonical_json(&value),
            Err(CanonicalError::OutOfRange {
                path: "$.count".into(),
                number: "9007199254740992".into()
            })
        );
    }

    // −(2^53) is refused: the lower bound is as tight as the upper.
    // Catches a bound placed one off on the negative side.
    #[test]
    fn minus_two_to_the_fifty_three_is_refused() {
        let value = json!([-9007199254740992_i64]);
        assert_eq!(
            canonical_json(&value),
            Err(CanonicalError::OutOfRange {
                path: "$[0]".into(),
                number: "-9007199254740992".into()
            })
        );
    }

    // The most negative i64 has no absolute value in i64, so a bound
    // checked through `abs` panics or wraps on it. It is refused like any
    // other integer out of range. Catches an overflowing bound check.
    #[test]
    fn the_most_negative_i64_is_refused() {
        let value = json!([i64::MIN]);
        assert_eq!(
            canonical_json(&value),
            Err(CanonicalError::OutOfRange {
                path: "$[0]".into(),
                number: "-9223372036854775808".into()
            })
        );
    }

    // Integers past i64 in either direction are refused as out of range,
    // not mistaken for floats. Catches a check that only looks at i64.
    #[test]
    fn an_integer_past_i64_is_out_of_range() {
        let value: Value = serde_json::from_str("[18446744073709551615]").expect("parse");
        assert_eq!(
            canonical_json(&value),
            Err(CanonicalError::OutOfRange {
                path: "$[0]".into(),
                number: "18446744073709551615".into()
            })
        );
        let value: Value = serde_json::from_str("[-9223372036854775809]").expect("parse");
        assert_eq!(
            canonical_json(&value),
            Err(CanonicalError::OutOfRange {
                path: "$[0]".into(),
                number: "-9223372036854775809".into()
            })
        );
    }

    // The integer token `-0` is zero, and RFC 8785 writes zero as `0`.
    // Catches `-0` refused as a float or written with its sign.
    #[test]
    fn minus_zero_is_written_as_zero() {
        let value: Value = serde_json::from_str("[-0]").expect("parse");
        assert_eq!(canonical(value), "[0]");
    }

    // A number with a fraction is refused, even a whole one, because the
    // ledger has no doubles. Catches a serializer that formats 2.0 as 2.
    #[test]
    fn a_float_is_refused() {
        let value: Value = serde_json::from_str(r#"{"a": [1, 2.0]}"#).expect("parse");
        assert_eq!(
            canonical_json(&value),
            Err(CanonicalError::NotAnInteger {
                path: "$.a[1]".into(),
                number: "2.0".into()
            })
        );
    }

    // Minus zero with a fraction is a float like any other. Catches a
    // `-0` rule that accepts every spelling of zero.
    #[test]
    fn minus_zero_with_a_fraction_is_refused() {
        let value: Value = serde_json::from_str("[-0.0]").expect("parse");
        assert_eq!(
            canonical_json(&value),
            Err(CanonicalError::NotAnInteger {
                path: "$[0]".into(),
                number: "-0.0".into()
            })
        );
    }

    // A number with an exponent is refused though its value is whole. The
    // number's text is serde_json's rendering of it, so only the refusal
    // and its path are asserted. Catches a check that tests the value
    // instead of the token.
    #[test]
    fn an_exponent_form_is_refused() {
        let value: Value = serde_json::from_str("[1e2]").expect("parse");
        assert!(matches!(
            canonical_json(&value),
            Err(CanonicalError::NotAnInteger { path, .. }) if path == "$[0]"
        ));
    }
}
