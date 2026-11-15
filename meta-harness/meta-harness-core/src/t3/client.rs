//! T3 Code's client protocol, Effect RPC JSON frames over one WebSocket on `/ws`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// The orchestration protocol this client speaks; T3 answers 426 to any other.
const ORCHESTRATION_PROTOCOL: u32 = 2;

enum Waiter {
    Call(oneshot::Sender<Result<Value, String>>),
    Stream(mpsc::UnboundedSender<Value>),
}

/// One authenticated connection to a T3 server.
#[derive(Clone)]
pub(crate) struct Client {
    out: mpsc::UnboundedSender<Message>,
    waiters: Arc<Mutex<HashMap<String, Waiter>>>,
    next: Arc<AtomicU64>,
}

impl Client {
    /// Connects to `origin` (`http://127.0.0.1:<port>`) with the owner's bearer token.
    pub async fn connect(origin: &str, token: &str) -> Result<Self, String> {
        let url = format!(
            "{}/ws?orchestrationProtocol={ORCHESTRATION_PROTOCOL}",
            origin.replacen("http", "ws", 1)
        );
        let mut request = url.into_client_request().map_err(|e| e.to_string())?;
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| "token is no header value".to_owned())?,
        );
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| format!("connect {origin}/ws: {e}"))?;
        let (mut sink, mut source) = socket.split();
        let (out, mut outgoing) = mpsc::unbounded_channel::<Message>();
        tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                if sink.send(message).await.is_err() {
                    break;
                }
            }
        });
        let waiters: Arc<Mutex<HashMap<String, Waiter>>> = Arc::default();
        let reader_waiters = waiters.clone();
        let ack = out.clone();
        tokio::spawn(async move {
            while let Some(Ok(message)) = source.next().await {
                let Message::Text(text) = message else {
                    continue;
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let frames = match value {
                    Value::Array(frames) => frames,
                    frame => vec![frame],
                };
                for frame in frames {
                    dispatch(&reader_waiters, &ack, frame).await;
                }
            }
            // The socket closed: every open call fails, every stream ends.
            for (_, waiter) in reader_waiters.lock().await.drain() {
                if let Waiter::Call(reply) = waiter {
                    let _ = reply.send(Err("T3 closed the connection".to_owned()));
                }
            }
        });
        Ok(Self {
            out,
            waiters,
            next: Arc::new(AtomicU64::new(1)),
        })
    }

    fn send_request(&self, id: &str, tag: &str, payload: Value) -> Result<(), String> {
        let frame =
            json!({"_tag": "Request", "id": id, "tag": tag, "payload": payload, "headers": []});
        self.out
            .send(Message::Text(frame.to_string().into()))
            .map_err(|_| "T3 connection is closed".to_owned())
    }

    /// Calls `tag` and returns its success value, or T3's failure as text.
    pub async fn call(&self, tag: &str, payload: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed).to_string();
        let (reply, answer) = oneshot::channel();
        self.waiters
            .lock()
            .await
            .insert(id.clone(), Waiter::Call(reply));
        self.send_request(&id, tag, payload)?;
        answer
            .await
            .map_err(|_| "T3 connection is closed".to_owned())?
    }

    /// Opens the stream `tag`; each received item arrives in order, already acknowledged.
    pub async fn stream(
        &self,
        tag: &str,
        payload: Value,
    ) -> Result<mpsc::UnboundedReceiver<Value>, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed).to_string();
        let (items, receiver) = mpsc::unbounded_channel();
        self.waiters
            .lock()
            .await
            .insert(id.clone(), Waiter::Stream(items));
        self.send_request(&id, tag, payload)?;
        Ok(receiver)
    }
}

async fn dispatch(
    waiters: &Mutex<HashMap<String, Waiter>>,
    ack: &mpsc::UnboundedSender<Message>,
    frame: Value,
) {
    let request_id = match &frame["requestId"] {
        Value::String(id) => id.clone(),
        Value::Number(id) => id.to_string(),
        _ => return,
    };
    match frame["_tag"].as_str() {
        Some("Chunk") => {
            // T3 waits for an Ack per chunk before it writes more (Effect RPC flow control).
            let _ = ack.send(Message::Text(
                json!({"_tag": "Ack", "requestId": request_id})
                    .to_string()
                    .into(),
            ));
            if let Some(Waiter::Stream(items)) = waiters.lock().await.get(&request_id)
                && let Some(values) = frame["values"].as_array()
            {
                for value in values {
                    let _ = items.send(value.clone());
                }
            }
        }
        Some("Exit") => {
            let result = if frame["exit"]["_tag"] == "Success" {
                Ok(frame["exit"]["value"].clone())
            } else {
                Err(frame["exit"]["cause"].to_string())
            };
            if let Some(Waiter::Call(reply)) = waiters.lock().await.remove(&request_id) {
                let _ = reply.send(result);
            }
        }
        _ => {}
    }
}
