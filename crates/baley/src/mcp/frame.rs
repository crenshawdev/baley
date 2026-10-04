//! Bounded line framing for the stdio transport.
//!
//! One frame is one line of JSON. The decoder keeps a frame only while it is
//! inside two bounds, 4 MiB before the newline and 128 levels of nesting, and
//! it does not end the input for any fault: Claude Code does not reconnect a
//! stdio server, so one bad frame must not cost the session its Baley. A frame
//! over a bound, or one that is not well-formed JSON, is discarded through its
//! newline and reported with the JSON-RPC id read before the fault, or none.
//! The next frame decodes normally.

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt};

/// The most bytes a frame may hold before its newline. A `\r` counts.
pub const MAX_FRAME_BYTES: usize = 4_194_304;

/// The deepest nesting of arrays and objects a frame may hold.
pub const MAX_DEPTH: usize = 128;

/// The longest id, as written on the wire, a fault answer repeats. A string id
/// over this cannot form a call identity anyway, so the answer carries null.
const MAX_ID_BYTES: usize = 256;

/// JSON-RPC's code for text that is not JSON.
const PARSE_ERROR: i64 = -32700;

/// JSON-RPC's code for JSON that is not a usable request.
const INVALID_REQUEST: i64 = -32600;

/// Bytes read from the source at a time.
const CHUNK: usize = 16 * 1024;

/// What was wrong with a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// More than [`MAX_FRAME_BYTES`] before the newline.
    TooLarge,
    /// Nested deeper than [`MAX_DEPTH`].
    TooDeep,
    /// Inside the bounds but not well-formed JSON.
    Malformed,
    /// Well-formed JSON the transport cannot read as a JSON-RPC message. The
    /// decoder never produces this; the transport builds it.
    NotAMessage,
}

/// A frame the decoder discarded.
#[derive(Debug, Clone, PartialEq)]
pub struct Fault {
    /// Which rule the frame broke.
    pub kind: FaultKind,
    /// The top-level `id` read before the fault, a string or an integer, or
    /// `None` when none was complete.
    pub id: Option<Value>,
}

/// What one byte did to the decoder.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Nothing is ready yet.
    More,
    /// A whole frame, without its newline.
    Frame(Vec<u8>),
    /// A frame was discarded. Later bytes are skipped through the next newline.
    Fault(Fault),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Open {
    Object,
    Array,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    Value,
    ValueOrEnd,
    KeyOrEnd,
    Key,
    Colon,
    CommaOrEnd,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Escape {
    Plain,
    Slash,
    Hex(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    None,
    Str { key: bool, escape: Escape },
    Atom { start: usize },
}

/// The byte state machine. It checks JSON syntax, counts bytes and depth, and
/// notes the top-level `id`. It holds the frame it is reading and nothing else.
#[derive(Debug)]
pub struct Decoder {
    frame: Vec<u8>,
    stack: Vec<Open>,
    expect: Expect,
    token: Token,
    started: bool,
    skipping: bool,
    key_len: usize,
    key_head: [u8; 2],
    key_is_id: bool,
    id_slot: bool,
    id_start: Option<usize>,
    id: Option<Value>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// A decoder at the start of a frame.
    pub fn new() -> Self {
        Self {
            frame: Vec::new(),
            stack: Vec::new(),
            expect: Expect::Value,
            token: Token::None,
            started: false,
            skipping: false,
            key_len: 0,
            key_head: [0; 2],
            key_is_id: false,
            id_slot: false,
            id_start: None,
            id: None,
        }
    }

    /// Feeds one byte.
    pub fn push(&mut self, byte: u8) -> Step {
        if self.skipping {
            if byte == b'\n' {
                self.skipping = false;
            }
            return Step::More;
        }
        if byte == b'\n' {
            return self.end_of_line();
        }
        if self.frame.len() == MAX_FRAME_BYTES {
            return self.fault(FaultKind::TooLarge, true);
        }
        self.frame.push(byte);
        match self.advance(byte) {
            Ok(()) => Step::More,
            Err(kind) => self.fault(kind, true),
        }
    }

    fn fault(&mut self, kind: FaultKind, skip: bool) -> Step {
        let id = self.id.take();
        *self = Self::new();
        self.skipping = skip;
        Step::Fault(Fault { kind, id })
    }

    fn end_of_line(&mut self) -> Step {
        if !self.started {
            self.frame.clear();
            return Step::More;
        }
        if let Token::Atom { start } = self.token
            && let Err(kind) = self.finish_atom(start, self.frame.len())
        {
            return self.fault(kind, false);
        }
        let whole = self.token == Token::None
            && self.expect == Expect::Done
            && std::str::from_utf8(&self.frame).is_ok();
        if !whole {
            return self.fault(FaultKind::Malformed, false);
        }
        let frame = std::mem::take(&mut self.frame);
        *self = Self::new();
        Step::Frame(frame)
    }

    fn advance(&mut self, byte: u8) -> Result<(), FaultKind> {
        match self.token {
            Token::Str { key, escape } => return self.string_byte(byte, key, escape),
            Token::Atom { start } => {
                if !is_delimiter(byte) {
                    return Ok(());
                }
                self.finish_atom(start, self.frame.len() - 1)?;
            }
            Token::None => {}
        }
        if is_space(byte) {
            return Ok(());
        }
        self.started = true;
        self.structural(byte)
    }

    fn structural(&mut self, byte: u8) -> Result<(), FaultKind> {
        match (self.expect, byte) {
            (Expect::Value, _) => self.start_value(byte),
            (Expect::ValueOrEnd, b']') | (Expect::KeyOrEnd, b'}') => self.close(),
            (Expect::ValueOrEnd, _) => self.start_value(byte),
            (Expect::KeyOrEnd | Expect::Key, b'"') => {
                self.key_len = 0;
                self.key_head = [0; 2];
                self.token = Token::Str {
                    key: true,
                    escape: Escape::Plain,
                };
                Ok(())
            }
            (Expect::Colon, b':') => {
                self.id_slot = self.key_is_id && self.id.is_none();
                self.expect = Expect::Value;
                Ok(())
            }
            (Expect::CommaOrEnd, b',') => {
                self.expect = match self.stack.last() {
                    Some(Open::Object) => Expect::Key,
                    _ => Expect::Value,
                };
                Ok(())
            }
            (Expect::CommaOrEnd, b'}') if self.stack.last() == Some(&Open::Object) => self.close(),
            (Expect::CommaOrEnd, b']') if self.stack.last() == Some(&Open::Array) => self.close(),
            _ => Err(FaultKind::Malformed),
        }
    }

    fn start_value(&mut self, byte: u8) -> Result<(), FaultKind> {
        let here = self.frame.len() - 1;
        let id_slot = std::mem::take(&mut self.id_slot);
        match byte {
            b'{' | b'[' => {
                if self.stack.len() == MAX_DEPTH {
                    return Err(FaultKind::TooDeep);
                }
                if byte == b'{' {
                    self.stack.push(Open::Object);
                    self.expect = Expect::KeyOrEnd;
                } else {
                    self.stack.push(Open::Array);
                    self.expect = Expect::ValueOrEnd;
                }
                Ok(())
            }
            b'"' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n' => {
                if id_slot {
                    self.id_start = Some(here);
                }
                self.token = if byte == b'"' {
                    Token::Str {
                        key: false,
                        escape: Escape::Plain,
                    }
                } else {
                    Token::Atom { start: here }
                };
                Ok(())
            }
            _ => Err(FaultKind::Malformed),
        }
    }

    fn string_byte(&mut self, byte: u8, key: bool, escape: Escape) -> Result<(), FaultKind> {
        let next = match (escape, byte) {
            (Escape::Plain, b'"') => {
                self.token = Token::None;
                return self.string_done(key);
            }
            (Escape::Plain, b'\\') => Escape::Slash,
            (Escape::Plain, 0..=31) => return Err(FaultKind::Malformed),
            (Escape::Plain, _) => {
                if key
                    && self.stack.len() == 1
                    && let Some(slot) = self.key_head.get_mut(self.key_len)
                {
                    *slot = byte;
                }
                Escape::Plain
            }
            (Escape::Slash, b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {
                Escape::Plain
            }
            (Escape::Slash, b'u') => Escape::Hex(4),
            (Escape::Hex(left), digit) if digit.is_ascii_hexdigit() => {
                if left == 1 {
                    Escape::Plain
                } else {
                    Escape::Hex(left - 1)
                }
            }
            _ => return Err(FaultKind::Malformed),
        };
        if key {
            // Every byte of a key counts, escapes included, so only the plain
            // two-byte spelling `id` is read as the id's key.
            self.key_len = self.key_len.saturating_add(1);
        }
        self.token = Token::Str { key, escape: next };
        Ok(())
    }

    fn string_done(&mut self, key: bool) -> Result<(), FaultKind> {
        if key {
            self.key_is_id = self.key_len == 2 && self.key_head == *b"id";
            self.expect = Expect::Colon;
            return Ok(());
        }
        self.value_done(self.frame.len());
        Ok(())
    }

    fn finish_atom(&mut self, start: usize, end: usize) -> Result<(), FaultKind> {
        if !valid_atom(&self.frame[start..end]) {
            return Err(FaultKind::Malformed);
        }
        self.token = Token::None;
        self.value_done(end);
        Ok(())
    }

    fn close(&mut self) -> Result<(), FaultKind> {
        self.stack.pop();
        self.value_done(self.frame.len());
        Ok(())
    }

    /// A value ended at `end`, exclusive. Reads the id if this was its value.
    fn value_done(&mut self, end: usize) {
        if let Some(start) = self.id_start.take()
            && end - start <= MAX_ID_BYTES
        {
            self.id = id_value(&self.frame[start..end]);
        }
        self.expect = if self.stack.is_empty() {
            Expect::Done
        } else {
            Expect::CommaOrEnd
        };
    }
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r')
}

fn is_delimiter(byte: u8) -> bool {
    is_space(byte) || matches!(byte, b',' | b']' | b'}')
}

fn id_value(text: &[u8]) -> Option<Value> {
    match serde_json::from_slice::<Value>(text).ok()? {
        value @ Value::String(_) => Some(value),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(Value::Number(number)),
        _ => None,
    }
}

fn valid_atom(text: &[u8]) -> bool {
    matches!(text, b"true" | b"false" | b"null") || valid_number(text)
}

fn valid_number(text: &[u8]) -> bool {
    let digits = |from: usize| {
        text[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let mut at = usize::from(text.first() == Some(&b'-'));
    match text.get(at) {
        Some(b'0') => at += 1,
        Some(b'1'..=b'9') => at += digits(at),
        _ => return false,
    }
    if text.get(at) == Some(&b'.') {
        let count = digits(at + 1);
        if count == 0 {
            return false;
        }
        at += 1 + count;
    }
    if matches!(text.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(text.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let count = digits(at);
        if count == 0 {
            return false;
        }
        at += count;
    }
    at == text.len()
}

/// What the reader found next on the input.
#[derive(Debug, PartialEq)]
pub enum Next {
    /// A whole frame, without its newline.
    Frame(Vec<u8>),
    /// A frame that was discarded.
    Fault(Fault),
    /// The input ended. A partial frame at the end is dropped.
    End,
}

/// Reads frames from any async byte source.
///
/// Partial progress lives in the struct, so a cancelled [`FrameReader::next`]
/// loses nothing: the next call carries on from the same byte.
pub struct FrameReader<R> {
    source: R,
    decoder: Decoder,
    buffer: Box<[u8; CHUNK]>,
    used: usize,
    at: usize,
    ended: bool,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    /// A reader over `source`.
    pub fn new(source: R) -> Self {
        Self {
            source,
            decoder: Decoder::new(),
            buffer: Box::new([0; CHUNK]),
            used: 0,
            at: 0,
            ended: false,
        }
    }

    /// The next frame, fault or end of input. A read error is the caller's.
    pub async fn next(&mut self) -> std::io::Result<Next> {
        loop {
            while self.at < self.used {
                let byte = self.buffer[self.at];
                self.at += 1;
                match self.decoder.push(byte) {
                    Step::More => {}
                    Step::Frame(frame) => return Ok(Next::Frame(frame)),
                    Step::Fault(fault) => return Ok(Next::Fault(fault)),
                }
            }
            if self.ended {
                return Ok(Next::End);
            }
            self.used = self.source.read(&mut self.buffer[..]).await?;
            self.at = 0;
            if self.used == 0 {
                self.ended = true;
            }
        }
    }
}

/// One JSON-RPC 2.0 error response for a discarded frame, carrying the id read
/// before the fault or `null`. The transport writes it as one line.
pub fn error_response(fault: &Fault) -> Value {
    let (code, message) = match fault.kind {
        FaultKind::TooLarge => (
            INVALID_REQUEST,
            format!("the frame is over the {MAX_FRAME_BYTES} byte limit"),
        ),
        FaultKind::TooDeep => (
            INVALID_REQUEST,
            format!("the frame nests deeper than {MAX_DEPTH} levels"),
        ),
        FaultKind::Malformed => (PARSE_ERROR, "the frame is not well-formed JSON".to_owned()),
        FaultKind::NotAMessage => (
            INVALID_REQUEST,
            "the frame is not a JSON-RPC message".to_owned(),
        ),
    };
    json!({
        "jsonrpc": "2.0",
        "id": fault.id.clone().unwrap_or(Value::Null),
        "error": {"code": code, "message": message},
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll, Waker};
    use tokio::io::ReadBuf;

    fn steps(wire: &[u8]) -> Vec<Step> {
        let mut decoder = Decoder::new();
        wire.iter()
            .filter_map(|byte| match decoder.push(*byte) {
                Step::More => None,
                step => Some(step),
            })
            .collect()
    }

    fn fault(kind: FaultKind, id: Option<Value>) -> Step {
        Step::Fault(Fault { kind, id })
    }

    /// `{"id":1,"pad":"xxx"}` padded to exactly `size` bytes.
    fn padded(size: usize) -> Vec<u8> {
        let head = br#"{"id":1,"pad":""#;
        let tail = br#""}"#;
        let mut wire = head.to_vec();
        wire.resize(size - tail.len(), b'x');
        wire.extend_from_slice(tail);
        assert_eq!(wire.len(), size);
        wire
    }

    fn nested(depth: usize) -> Vec<u8> {
        let mut wire = vec![b'['; depth];
        wire.extend(std::iter::repeat_n(b']', depth));
        wire
    }

    const PING: &[u8] = br#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#;

    fn line(frame: &[u8]) -> Vec<u8> {
        let mut wire = frame.to_vec();
        wire.push(b'\n');
        wire
    }

    fn is_frame(step: &Step, frame: &[u8]) -> bool {
        matches!(step, Step::Frame(got) if got == frame)
    }

    #[test]
    fn a_frame_of_exactly_the_byte_limit_is_refused() {
        let out = steps(&line(&padded(MAX_FRAME_BYTES)));
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], Step::Frame(frame) if frame.len() == MAX_FRAME_BYTES));
    }

    #[test]
    fn a_frame_one_byte_over_the_limit_is_accepted() {
        let out = steps(&line(&padded(MAX_FRAME_BYTES + 1)));
        assert_eq!(out, [fault(FaultKind::TooLarge, Some(json!(1)))]);
    }

    #[test]
    fn a_carriage_return_before_the_newline_does_not_count_toward_the_limit() {
        let mut wire = padded(MAX_FRAME_BYTES - 1);
        wire.push(b'\r');
        let out = steps(&line(&wire));
        assert!(matches!(&out[0], Step::Frame(frame) if frame.len() == MAX_FRAME_BYTES));
    }

    #[test]
    fn nesting_of_exactly_the_depth_limit_is_refused() {
        let out = steps(&line(&nested(MAX_DEPTH)));
        assert_eq!(out, [Step::Frame(nested(MAX_DEPTH))]);
    }

    #[test]
    fn nesting_one_level_past_the_depth_limit_is_accepted() {
        let out = steps(&line(&nested(MAX_DEPTH + 1)));
        assert_eq!(out, [fault(FaultKind::TooDeep, None)]);
    }

    #[test]
    fn brackets_inside_a_string_count_toward_depth() {
        let mut wire = br#"{"a":""#.to_vec();
        wire.extend(std::iter::repeat_n(b'[', 300));
        wire.extend_from_slice(br#"\"["}"#);
        let out = steps(&line(&wire));
        assert_eq!(out, [Step::Frame(wire)]);
    }

    #[test]
    fn the_frame_after_an_oversized_one_is_lost_or_corrupted() {
        let mut wire = line(&padded(MAX_FRAME_BYTES + 10));
        wire.extend(line(PING));
        let out = steps(&wire);
        assert_eq!(out.len(), 2);
        assert!(matches!(&out[0], Step::Fault(f) if f.kind == FaultKind::TooLarge));
        assert!(is_frame(&out[1], PING));
    }

    #[test]
    fn the_frame_after_a_too_deep_one_is_lost_or_corrupted() {
        let mut wire = line(&nested(MAX_DEPTH + 5));
        wire.extend(line(PING));
        let out = steps(&wire);
        assert_eq!(out.len(), 2);
        assert!(matches!(&out[0], Step::Fault(f) if f.kind == FaultKind::TooDeep));
        assert!(is_frame(&out[1], PING));
    }

    #[test]
    fn the_rest_of_a_faulted_line_is_read_as_a_frame_of_its_own() {
        let mut wire = br#"{"a":}"#.to_vec();
        wire.extend_from_slice(PING);
        wire.push(b'\n');
        wire.extend(line(PING));
        let out = steps(&wire);
        assert_eq!(out.len(), 2);
        assert!(matches!(&out[0], Step::Fault(_)));
        assert!(is_frame(&out[1], PING));
    }

    #[test]
    fn a_fault_that_lands_on_the_newline_swallows_the_next_frame() {
        let mut wire = line(br#"{"id":4,"a":"#);
        wire.extend(line(PING));
        let out = steps(&wire);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], fault(FaultKind::Malformed, Some(json!(4))));
        assert!(is_frame(&out[1], PING));
    }

    fn over_the_limit_after(prefix: &[u8]) -> Vec<u8> {
        let mut wire = prefix.to_vec();
        wire.resize(MAX_FRAME_BYTES + 2, b'x');
        wire.push(b'\n');
        wire
    }

    #[test]
    fn an_id_read_before_the_bound_is_left_out_of_the_fault() {
        let out = steps(&over_the_limit_after(br#"{"id":5,"pad":""#));
        assert_eq!(out, [fault(FaultKind::TooLarge, Some(json!(5)))]);
        let out = steps(&over_the_limit_after(br#"{"id":"call-7","pad":""#));
        assert_eq!(out, [fault(FaultKind::TooLarge, Some(json!("call-7")))]);
    }

    #[test]
    fn a_fault_with_no_id_read_is_given_a_made_up_one() {
        let out = steps(&over_the_limit_after(br#"{"pad":""#));
        assert_eq!(out, [fault(FaultKind::TooLarge, None)]);
        let mut late = br#"{"pad":""#.to_vec();
        late.resize(MAX_FRAME_BYTES - 20, b'x');
        late.extend_from_slice(br#"","pad2":""#);
        late.resize(MAX_FRAME_BYTES + 5, b'x');
        late.extend_from_slice(br#"","id":9}"#);
        late.push(b'\n');
        assert_eq!(steps(&late), [fault(FaultKind::TooLarge, None)]);
    }

    #[test]
    fn an_id_that_is_not_the_top_level_members_is_taken_for_the_calls() {
        let out = steps(&over_the_limit_after(br#"{"params":{"id":9},"pad":""#));
        assert_eq!(out, [fault(FaultKind::TooLarge, None)]);
        let out = steps(&over_the_limit_after(br#"{"id":{"a":1},"pad":""#));
        assert_eq!(out, [fault(FaultKind::TooLarge, None)]);
        let out = steps(&over_the_limit_after(br#"{"id":null,"pad":""#));
        assert_eq!(out, [fault(FaultKind::TooLarge, None)]);
    }

    #[test]
    fn an_id_still_open_at_the_bound_is_reported_as_if_complete() {
        let mut open = br#"{"id":"#.to_vec();
        open.push(b'"');
        open.resize(MAX_FRAME_BYTES + 3, b'x');
        open.push(b'\n');
        assert_eq!(steps(&open), [fault(FaultKind::TooLarge, None)]);
    }

    #[test]
    fn a_key_only_shaped_like_the_id_key_supplies_the_id() {
        let wire = over_the_limit_after(br#"{"id":null,"\n":5,"pad":""#);
        assert_eq!(steps(&wire), [fault(FaultKind::TooLarge, None)]);
        let wire = over_the_limit_after(concat!("{\"", "\\", "u0069d\":5,\"pad\":\"").as_bytes());
        assert_eq!(steps(&wire), [fault(FaultKind::TooLarge, None)]);
        let wire = over_the_limit_after(br#"{"idx":5,"i":6,"pad":""#);
        assert_eq!(steps(&wire), [fault(FaultKind::TooLarge, None)]);
    }

    #[test]
    fn an_id_over_the_echo_limit_is_repeated_in_the_answer() {
        let long = "a".repeat(MAX_ID_BYTES);
        let mut wire = format!(r#"{{"id":"{long}","pad":""#).into_bytes();
        wire.resize(MAX_FRAME_BYTES + 2, b'x');
        wire.push(b'\n');
        assert_eq!(steps(&wire), [fault(FaultKind::TooLarge, None)]);
        let short = "a".repeat(MAX_ID_BYTES - 2);
        let mut wire = format!(r#"{{"id":"{short}","pad":""#).into_bytes();
        wire.resize(MAX_FRAME_BYTES + 2, b'x');
        wire.push(b'\n');
        assert_eq!(
            steps(&wire),
            [fault(FaultKind::TooLarge, Some(json!(short)))]
        );
    }

    #[test]
    fn the_first_id_is_replaced_by_a_later_duplicate() {
        let wire = over_the_limit_after(br#"{"id":1,"id":2,"pad":""#);
        assert_eq!(steps(&wire), [fault(FaultKind::TooLarge, Some(json!(1)))]);
    }

    #[test]
    fn the_error_line_is_not_json_rpc_two_with_the_read_id() {
        let line = error_response(&Fault {
            kind: FaultKind::TooLarge,
            id: Some(json!(5)),
        });
        assert_eq!(line["jsonrpc"], json!("2.0"));
        assert_eq!(line["id"], json!(5));
        assert_eq!(line["error"]["code"], json!(-32600));
        assert!(
            line["error"]["message"]
                .as_str()
                .unwrap()
                .contains("4194304")
        );
        assert!(line.get("result").is_none());
        let line = error_response(&Fault {
            kind: FaultKind::TooDeep,
            id: Some(json!("abc")),
        });
        assert_eq!(line["id"], json!("abc"));
        assert!(line["error"]["message"].as_str().unwrap().contains("128"));
    }

    #[test]
    fn the_error_line_carries_an_id_when_none_was_read() {
        for kind in [
            FaultKind::TooLarge,
            FaultKind::TooDeep,
            FaultKind::Malformed,
            FaultKind::NotAMessage,
        ] {
            let line = error_response(&Fault { kind, id: None });
            assert_eq!(line["id"], Value::Null, "{kind:?}");
            assert!(line.as_object().unwrap().contains_key("id"));
        }
    }

    #[test]
    fn each_fault_kind_takes_its_own_json_rpc_code() {
        let code = |kind| error_response(&Fault { kind, id: None })["error"]["code"].clone();
        assert_eq!(code(FaultKind::Malformed), json!(-32700));
        assert_eq!(code(FaultKind::NotAMessage), json!(-32600));
        assert_eq!(code(FaultKind::TooLarge), json!(-32600));
        assert_eq!(code(FaultKind::TooDeep), json!(-32600));
    }

    #[test]
    fn a_malformed_frame_inside_the_bounds_is_reported_as_a_parse_error_and_skipped() {
        let bad: [&[u8]; 14] = [
            br#"{"a":}"#,
            br#"{"a":1,}"#,
            br#"[1,]"#,
            br#"{"a" 1}"#,
            br#"{a:1}"#,
            br#"{"a":"\x"}"#,
            br#"{"a":"\u12G4"}"#,
            br#"{"a":01}"#,
            br#"{"a":1.}"#,
            br#"{"a":tru}"#,
            br#"{}{}"#,
            br#"{"a":1}x"#,
            b"{\"a\":\"\x01\"}",
            b"{\"a\":\"\xff\"}",
        ];
        for wire in bad {
            let mut input = line(wire);
            input.extend(line(PING));
            let out = steps(&input);
            assert_eq!(out.len(), 2, "{}", String::from_utf8_lossy(wire));
            assert_eq!(out[0], fault(FaultKind::Malformed, None));
            assert!(is_frame(&out[1], PING));
        }
    }

    #[test]
    fn a_frame_cut_off_by_its_newline_is_a_parse_error() {
        for wire in [&br#"{"a":"#[..], br#"{"a":"abc"#, br#"[1,2"#, br#"{"a":1"#] {
            let out = steps(&line(wire));
            assert_eq!(out, [fault(FaultKind::Malformed, None)]);
        }
    }

    #[test]
    fn a_malformed_frame_reports_the_id_read_before_it() {
        let out = steps(&line(br#"{"id":3,"method":}"#));
        assert_eq!(out, [fault(FaultKind::Malformed, Some(json!(3)))]);
    }

    #[test]
    fn well_formed_json_is_never_refused_as_malformed() {
        let good: [&[u8]; 9] = [
            br#"{}"#,
            br#"[]"#,
            br#" {"a" : [1, -2.5e+10, 0, 1E3, true, false, null, "x"] , "b":{}} "#,
            concat!("{\"a\":\"", "\\", "u00e9 ", "\\", "uD83D", "\\", "uDE00\"}").as_bytes(),
            "{\"a\":\"caf\u{e9} \u{1F600}\"}".as_bytes(),
            br#"{"id":-12}"#,
            br#"5"#,
            br#""text""#,
            br#"true"#,
        ];
        for wire in good {
            assert_eq!(
                steps(&line(wire)),
                [Step::Frame(wire.to_vec())],
                "{}",
                String::from_utf8_lossy(wire)
            );
        }
    }

    #[test]
    fn blank_lines_between_frames_are_skipped() {
        let mut wire = b"\n \t\r\n".to_vec();
        wire.extend(line(PING));
        wire.extend(b"\n\n");
        wire.extend(line(PING));
        let out = steps(&wire);
        assert_eq!(out.len(), 2);
        assert!(is_frame(&out[0], PING));
        assert!(is_frame(&out[1], PING));
    }

    #[test]
    fn a_crlf_frame_is_returned_whole() {
        let mut wire = PING.to_vec();
        wire.extend_from_slice(b"\r\n");
        let out = steps(&wire);
        let mut whole = PING.to_vec();
        whole.push(b'\r');
        assert_eq!(out, [Step::Frame(whole)]);
    }

    async fn read_all(wire: &[u8]) -> Vec<Next> {
        let mut reader = FrameReader::new(wire);
        let mut out = Vec::new();
        loop {
            let next = reader.next().await.unwrap();
            let end = next == Next::End;
            out.push(next);
            if end {
                return out;
            }
        }
    }

    #[tokio::test]
    async fn a_malformed_frame_ends_input_instead_of_being_answered() {
        let mut wire = line(br#"{"id":8,"a":}"#);
        wire.extend(line(PING));
        let out = read_all(&wire).await;
        assert_eq!(
            out,
            [
                Next::Fault(Fault {
                    kind: FaultKind::Malformed,
                    id: Some(json!(8))
                }),
                Next::Frame(PING.to_vec()),
                Next::End,
            ]
        );
    }

    #[tokio::test]
    async fn an_oversized_frame_ends_input_instead_of_being_answered() {
        let mut wire = line(&padded(MAX_FRAME_BYTES + 1));
        wire.extend(line(PING));
        let out = read_all(&wire).await;
        assert_eq!(out.len(), 3);
        assert!(matches!(&out[0], Next::Fault(f) if f.kind == FaultKind::TooLarge));
        assert_eq!(out[1], Next::Frame(PING.to_vec()));
        assert_eq!(out[2], Next::End);
    }

    #[tokio::test]
    async fn a_partial_frame_at_end_of_input_is_returned_as_a_frame() {
        let out = read_all(br#"{"method":"ping""#).await;
        assert_eq!(out, [Next::End]);
    }

    #[tokio::test]
    async fn a_second_frame_already_buffered_is_returned_corrupted() {
        let mut wire = line(PING);
        wire.extend(line(br#"{"id":3}"#));
        let out = read_all(&wire).await;
        assert_eq!(
            out,
            [
                Next::Frame(PING.to_vec()),
                Next::Frame(br#"{"id":3}"#.to_vec()),
                Next::End
            ]
        );
    }

    #[tokio::test]
    async fn end_of_input_is_reported_again_on_every_later_read() {
        let mut reader = FrameReader::new(&b""[..]);
        for _ in 0..3 {
            assert_eq!(reader.next().await.unwrap(), Next::End);
        }
    }

    /// Yields its bytes up to `pause_at`, then one `Pending`, then the rest.
    struct Paused {
        bytes: Vec<u8>,
        at: usize,
        pause_at: Option<usize>,
    }

    impl AsyncRead for Paused {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            out: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.pause_at == Some(self.at) {
                self.pause_at = None;
                return Poll::Pending;
            }
            let stop = self.pause_at.unwrap_or(self.bytes.len());
            let count = out.remaining().min(stop - self.at);
            out.put_slice(&self.bytes[self.at..self.at + count]);
            self.at += count;
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn a_cancelled_read_loses_the_bytes_it_already_took() {
        let source = Paused {
            bytes: line(PING),
            at: 0,
            pause_at: Some(12),
        };
        let mut reader = FrameReader::new(source);
        let mut future = Box::pin(reader.next());
        let mut context = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut context).is_pending());
        drop(future);
        assert_eq!(reader.next().await.unwrap(), Next::Frame(PING.to_vec()));
    }
}
