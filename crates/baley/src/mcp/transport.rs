//! Carries frames between stdio and rmcp through the bounded decoder.
//!
//! What to do with each thing the reader returns is a pure decision in
//! [`decide`]. The transport around it only reads, writes and signals. It does
//! no admission and no queueing: those happen after rmcp has validated a
//! request, in the handler. It sizes the one request that will be queued and
//! hands that size on as a request extension.

use std::io;
use std::sync::Arc;

use rmcp::RoleServer;
use rmcp::model::{ClientRequest, JsonRpcMessage};
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex, watch};

use super::frame::{Fault, FaultKind, FrameReader, Next, error_response};

/// The longest string id the decoder reads for an error line. A longer one
/// cannot name a call, so its error goes out with a null id.
const MAX_ID_BYTES: usize = 256;

/// The raw size of a `tools/call` frame, without its newline. The transport
/// attaches it as a request extension, and the handler reads it from the
/// request context to charge the queue's byte cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawFrameBytes(pub usize);

/// How the input came to an end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEnd {
    /// The client closed its end.
    Closed,
    /// Reading failed.
    Failed,
}

/// What to do with one thing the reader returned.
#[derive(Debug)]
pub enum Decision {
    /// Hand this message to rmcp.
    Deliver(Box<RxJsonRpcMessage<RoleServer>>),
    /// Write this error line and keep reading.
    Reply(Value),
    /// No more input.
    End(InputEnd),
}

/// Decides what a read result means. A fault is answered and reading goes on:
/// Claude Code does not restart a stdio server, so one bad frame from one
/// subagent must not leave the whole session without Baley.
pub fn decide(read: io::Result<Next>) -> Decision {
    match read {
        Err(_) => Decision::End(InputEnd::Failed),
        Ok(Next::End) => Decision::End(InputEnd::Closed),
        Ok(Next::Fault(fault)) => Decision::Reply(error_response(&fault)),
        Ok(Next::Frame(frame)) => {
            match serde_json::from_slice::<RxJsonRpcMessage<RoleServer>>(&frame) {
                Ok(mut message) => {
                    attach_size(&mut message, frame.len());
                    Decision::Deliver(Box::new(message))
                }
                Err(_) => Decision::Reply(error_response(&Fault {
                    kind: FaultKind::NotAMessage,
                    id: id_of(&frame),
                })),
            }
        }
    }
}

/// Puts the frame's size on a `tools/call`, the one request that enters the
/// queue. Any other message carries none.
fn attach_size(message: &mut RxJsonRpcMessage<RoleServer>, bytes: usize) {
    if let JsonRpcMessage::Request(envelope) = message
        && let ClientRequest::CallToolRequest(request) = &mut envelope.request
    {
        request.extensions.insert(RawFrameBytes(bytes));
    }
}

/// The request id of a well-formed frame that is not a message, so the error
/// can name the call it answers.
fn id_of(frame: &[u8]) -> Option<Value> {
    let value: Value = serde_json::from_slice(frame).ok()?;
    match value.get("id")? {
        id @ Value::String(text) if text.len() <= MAX_ID_BYTES => Some(id.clone()),
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            Some(Value::Number(number.clone()))
        }
        _ => None,
    }
}

/// The orchestrator's side of a [`StdioTransport`].
pub struct Control {
    closing: watch::Sender<bool>,
    ended: watch::Receiver<Option<InputEnd>>,
}

impl Control {
    /// Stops reading. The transport's next `receive` returns `None` without
    /// reading another frame.
    pub fn close_admission(&self) {
        self.closing.send_replace(true);
    }

    /// How the input ended, once it has.
    pub fn ended(&self) -> Option<InputEnd> {
        *self.ended.borrow()
    }

    /// Waits until the input ends on its own. It never returns after
    /// [`Control::close_admission`] alone.
    pub async fn input_ended(&mut self) -> InputEnd {
        match self.ended.wait_for(Option::is_some).await {
            Ok(end) => end.unwrap_or(InputEnd::Closed),
            // The transport is gone, which only happens once it stopped reading.
            Err(_) => InputEnd::Closed,
        }
    }
}

/// An rmcp server transport over any byte source and sink.
pub struct StdioTransport<R, W> {
    reader: FrameReader<R>,
    // Responses and error lines share one writer so lines never interleave.
    writer: Arc<Mutex<W>>,
    closing: watch::Receiver<bool>,
    ended: watch::Sender<Option<InputEnd>>,
}

impl<R: AsyncRead + Unpin, W> StdioTransport<R, W> {
    /// A transport over `source` and `sink`, and the handle that drives it.
    pub fn new(source: R, sink: W) -> (Self, Control) {
        let (closing_tx, closing) = watch::channel(false);
        let (ended, ended_rx) = watch::channel(None);
        (
            Self {
                reader: FrameReader::new(source),
                writer: Arc::new(Mutex::new(sink)),
                closing,
                ended,
            },
            Control {
                closing: closing_tx,
                ended: ended_rx,
            },
        )
    }
}

async fn write_line<W: AsyncWrite + Unpin>(writer: &Mutex<W>, line: &Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(line).map_err(io::Error::other)?;
    bytes.push(b'\n');
    let mut sink = writer.lock().await;
    sink.write_all(&bytes).await?;
    sink.flush().await
}

impl<R, W> Transport<RoleServer> for StdioTransport<R, W>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Error = io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send + 'static {
        let writer = Arc::clone(&self.writer);
        let line = serde_json::to_value(&item).map_err(io::Error::other);
        async move { write_line(&writer, &line?).await }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        loop {
            // A frame read after admission closed would be answered by nobody.
            let read = tokio::select! {
                biased;
                _ = self.closing.wait_for(|closing| *closing) => return None,
                read = self.reader.next() => read,
            };
            if let Err(error) = &read {
                eprintln!("baley: input failed: {error}");
            }
            match decide(read) {
                Decision::Deliver(message) => return Some(*message),
                Decision::Reply(line) => {
                    if let Err(error) = write_line(&self.writer, &line).await {
                        eprintln!("baley: output failed: {error}");
                        self.ended.send_replace(Some(InputEnd::Failed));
                        return None;
                    }
                }
                Decision::End(end) => {
                    self.ended.send_replace(Some(end));
                    return None;
                }
            }
        }
    }

    async fn close(&mut self) -> io::Result<()> {
        self.writer.lock().await.shutdown().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame(value: Value) -> io::Result<Next> {
        Ok(Next::Frame(serde_json::to_vec(&value).unwrap()))
    }

    fn call() -> Value {
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"baley_version","arguments":{}}})
    }

    fn reply(decision: Decision) -> Value {
        match decision {
            Decision::Reply(line) => line,
            other => panic!("expected an error line, got {other:?}"),
        }
    }

    fn delivered(decision: Decision) -> RxJsonRpcMessage<RoleServer> {
        match decision {
            Decision::Deliver(message) => *message,
            other => panic!("expected a delivered message, got {other:?}"),
        }
    }

    fn size_of(message: &mut RxJsonRpcMessage<RoleServer>) -> Option<RawFrameBytes> {
        match message {
            JsonRpcMessage::Request(envelope) => match &mut envelope.request {
                ClientRequest::CallToolRequest(request) => {
                    request.extensions.get::<RawFrameBytes>().copied()
                }
                ClientRequest::ListToolsRequest(request) => {
                    request.extensions.get::<RawFrameBytes>().copied()
                }
                ClientRequest::PingRequest(request) => {
                    request.extensions.get::<RawFrameBytes>().copied()
                }
                other => panic!("unexpected request {other:?}"),
            },
            other => panic!("unexpected message {other:?}"),
        }
    }

    #[test]
    fn a_bound_fault_ends_input_instead_of_being_answered() {
        for kind in [FaultKind::TooLarge, FaultKind::TooDeep] {
            let line = reply(decide(Ok(Next::Fault(Fault {
                kind,
                id: Some(json!(7)),
            }))));
            assert_eq!(line["id"], 7);
            assert_eq!(line["error"]["code"], -32600);
        }
    }

    #[test]
    fn a_malformed_frame_ends_input_instead_of_getting_a_parse_error() {
        let line = reply(decide(Ok(Next::Fault(Fault {
            kind: FaultKind::Malformed,
            id: None,
        }))));
        assert_eq!(line["id"], Value::Null);
        assert_eq!(line["error"]["code"], -32700);
    }

    #[test]
    fn well_formed_json_that_is_not_a_message_ends_input_or_goes_unanswered() {
        let line = reply(decide(frame(json!({"jsonrpc":"2.0","id":7,"method":5}))));
        assert_eq!(line["error"]["code"], -32600);
        assert_eq!(line["id"], 7);
        for text in [json!({}), json!([1, 2]), json!("text"), json!(3)] {
            let line = reply(decide(frame(text)));
            assert_eq!(line["error"]["code"], -32600);
            assert_eq!(line["id"], Value::Null);
        }
    }

    #[test]
    fn a_non_message_with_an_over_long_string_id_is_answered_with_a_null_id() {
        let long = "x".repeat(MAX_ID_BYTES + 1);
        let line = reply(decide(frame(json!({"jsonrpc":"2.0","id":long,"method":5}))));
        assert_eq!(line["id"], Value::Null);
    }

    #[test]
    fn a_tools_call_is_delivered_without_its_byte_count() {
        let bytes = serde_json::to_vec(&call()).unwrap();
        let mut message = delivered(decide(Ok(Next::Frame(bytes.clone()))));
        assert_eq!(size_of(&mut message), Some(RawFrameBytes(bytes.len())));
    }

    #[test]
    fn the_byte_count_is_the_frame_length_and_not_the_message_it_decodes_to() {
        // Extra spaces make the raw frame longer than any re-encoding of it.
        let raw = br#"{ "jsonrpc" : "2.0" , "id" : 1 , "method" : "tools/call" , "params" : { "name" : "baley_version" , "arguments" : { } } }"#;
        let mut message = delivered(decide(Ok(Next::Frame(raw.to_vec()))));
        assert_eq!(size_of(&mut message), Some(RawFrameBytes(raw.len())));
    }

    #[test]
    fn tools_list_and_ping_requests_carry_no_byte_count() {
        for body in [
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":3,"method":"ping"}),
        ] {
            let mut message = delivered(decide(frame(body)));
            assert_eq!(size_of(&mut message), None);
        }
    }

    #[test]
    fn end_of_input_is_reported_as_closed_and_a_read_error_as_failed() {
        assert!(matches!(
            decide(Ok(Next::End)),
            Decision::End(InputEnd::Closed)
        ));
        assert!(matches!(
            decide(Err(io::Error::other("broken pipe"))),
            Decision::End(InputEnd::Failed)
        ));
    }
}
