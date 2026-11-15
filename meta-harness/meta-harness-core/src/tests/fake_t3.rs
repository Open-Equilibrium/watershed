//! A stand-in for T3's WebSocket: it answers each request with the frames a test scripts, records
//! the requests, and sends the chunks a test pushes. No T3 process runs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// A text frame carrying `value`.
pub fn frame(value: Value) -> Message {
    Message::Text(value.to_string().into())
}

/// The frame that ends request `id` with `value`.
pub fn ok(id: &str, value: Value) -> Message {
    frame(json!({"_tag": "Exit", "requestId": id, "exit": {"_tag": "Success", "value": value}}))
}

/// The frame that ends request `id` with the failure `cause`.
pub fn fail(id: &str, cause: Value) -> Message {
    frame(json!({"_tag": "Exit", "requestId": id, "exit": {"_tag": "Failure", "cause": cause}}))
}

/// The frame with stream items for request `id`.
pub fn chunk(id: &str, values: Vec<Value>) -> Message {
    frame(json!({"_tag": "Chunk", "requestId": id, "values": values}))
}

enum Out {
    Chunk(String, Vec<Value>),
    Close,
}

/// A running stand-in on a loopback port.
pub struct Fake {
    /// `http://127.0.0.1:<port>`, as T3's origin is.
    pub origin: String,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    out: mpsc::UnboundedSender<Out>,
}

impl Fake {
    /// Accepts one connection and answers each request with `answer(tag, payload, id)`.
    pub async fn start(
        answer: impl Fn(&str, &Value, &str) -> Vec<Message> + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        // The latest request id per tag, which pushed chunks answer.
        let latest: Arc<Mutex<HashMap<String, String>>> = Arc::default();
        let (out, mut outgoing) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            // Requests keep being answered after the handle is dropped; only `close` ends them.
            let mut handle_open = true;
            loop {
                tokio::select! {
                    incoming = socket.next() => {
                        let Some(Ok(Message::Text(text))) = incoming else { break };
                        let frames = match serde_json::from_str::<Value>(&text).unwrap() {
                            Value::Array(frames) => frames,
                            frame => vec![frame],
                        };
                        for request in frames.iter().filter(|f| f["_tag"] == "Request") {
                            let id = request["id"].as_str().unwrap().to_owned();
                            let tag = request["tag"].as_str().unwrap().to_owned();
                            let payload = request["payload"].clone();
                            latest.lock().unwrap().insert(tag.clone(), id.clone());
                            log.lock().unwrap().push((tag.clone(), payload.clone()));
                            for reply in answer(&tag, &payload, &id) {
                                socket.send(reply).await.unwrap();
                            }
                        }
                    }
                    next = outgoing.recv(), if handle_open => match next {
                        Some(Out::Chunk(tag, values)) => {
                            let id = latest.lock().unwrap()[&tag].clone();
                            socket.send(chunk(&id, values)).await.unwrap();
                        }
                        Some(Out::Close) => {
                            let _ = socket.close(None).await;
                            break;
                        }
                        None => handle_open = false,
                    },
                }
            }
        });
        Self {
            origin,
            requests,
            out,
        }
    }

    /// Sends `values` on the latest stream of `tag`, as T3 sends each change.
    pub fn push(&self, tag: &str, values: Vec<Value>) {
        let _ = self.out.send(Out::Chunk(tag.to_owned(), values));
    }

    /// Closes the connection, as T3 does when it stops.
    pub fn close(&self) {
        let _ = self.out.send(Out::Close);
    }

    /// Waits until T3 received `count` requests for `tag`, so chunks for them can follow.
    pub async fn received(&self, tag: &str, count: usize) {
        while self.payloads(tag).len() < count {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    /// The payloads of the requests for `tag`, oldest first.
    pub fn payloads(&self, tag: &str) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| t == tag)
            .map(|(_, payload)| payload.clone())
            .collect()
    }
}
