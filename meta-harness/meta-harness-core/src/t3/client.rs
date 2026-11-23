use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::util::{TIMEOUT, lock};

const ORCHESTRATION_PROTOCOL: u32 = 2;

const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(200),
    Duration::from_millis(500),
    Duration::from_secs(1),
];

const LOST_CALL: &str = "T3 closed the connection before it answered; the call was not sent again";

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

type Issue = Arc<dyn Fn() -> BoxFuture<'static, Result<String, String>> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Failure {
    Refused(String),
    ConnectionLost,
}

type Slot = Arc<Mutex<Option<Failure>>>;

enum Waiter {
    Call(oneshot::Sender<Result<Value, String>>),
    Stream {
        items: mpsc::UnboundedSender<Item>,
        frame: Option<Value>,
        failure: Slot,
    },
}

struct Item {
    value: Value,
    ack: Option<mpsc::UnboundedSender<Message>>,
}

#[derive(Default)]
struct State {
    out: Option<mpsc::UnboundedSender<Message>>,
    waiters: HashMap<String, Waiter>,
    closed: bool,
    stopping: bool,
}

type Link = Arc<Mutex<State>>;

#[derive(Clone)]
pub(crate) struct Client {
    link: Link,
    next: Arc<AtomicU64>,
}

struct Waiting {
    link: Link,
    id: String,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        lock(&self.link).waiters.remove(&self.id);
    }
}

pub(crate) struct Subscription {
    items: mpsc::UnboundedReceiver<Item>,
    id: String,
    link: Link,
    failure: Slot,
}

impl Subscription {
    pub fn failure(&self) -> Option<Failure> {
        lock(&self.failure).clone()
    }

    pub async fn recv(&mut self) -> Option<Value> {
        let item = self.items.recv().await?;
        if let Some(ack) = item.ack {
            let _ = ack.send(frame_message(&json!({"_tag": "Ack", "requestId": self.id})));
        }
        Some(item.value)
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut state = lock(&self.link);
        let open = state.waiters.remove(&self.id).is_some();
        if let (true, Some(out)) = (open, &state.out) {
            let frame = json!({"_tag": "Interrupt", "requestId": self.id});
            let _ = out.send(frame_message(&frame));
        }
    }
}

impl Client {
    pub async fn connect(
        origin: &str,
        issue: impl Fn() -> BoxFuture<'static, Result<String, String>> + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let issue: Issue = Arc::new(issue);
        let socket = open(origin, &issue).await?;
        let link: Link = Arc::default();
        let served = install(&link, socket);
        tokio::spawn(keep(link.clone(), origin.to_owned(), issue, served));
        Ok(Self {
            link,
            next: Arc::new(AtomicU64::new(1)),
        })
    }

    fn request(&self, id: &str, frame: &Value, waiter: Waiter) -> Result<(), String> {
        let mut state = lock(&self.link);
        if state.closed {
            return Err("T3 closed the connection".to_owned());
        }
        let Some(out) = &state.out else {
            return Err("T3 is reconnecting; try again".to_owned());
        };
        out.send(frame_message(frame))
            .map_err(|_| "T3 connection is closed".to_owned())?;
        state.waiters.insert(id.to_owned(), waiter);
        Ok(())
    }

    fn next_id(&self) -> String {
        self.next.fetch_add(1, Ordering::Relaxed).to_string()
    }

    pub async fn call(&self, tag: &str, payload: Value) -> Result<Value, String> {
        let id = self.next_id();
        let (reply, answer) = oneshot::channel();
        self.request(&id, &request_frame(&id, tag, payload), Waiter::Call(reply))?;
        let _waiter = Waiting {
            link: self.link.clone(),
            id,
        };
        answer
            .await
            .map_err(|_| "T3 connection is closed".to_owned())?
    }

    pub async fn command(&self, tag: &str, mut payload: Value) -> Result<Value, String> {
        payload["commandId"] = json!(super::fresh_id("cmd"));
        self.call(tag, payload).await
    }

    pub async fn stream(&self, tag: &str, payload: Value) -> Result<Subscription, String> {
        self.open_stream(tag, payload, true)
    }

    pub async fn action(&self, tag: &str, payload: Value) -> Result<Subscription, String> {
        self.open_stream(tag, payload, false)
    }

    fn open_stream(&self, tag: &str, payload: Value, resend: bool) -> Result<Subscription, String> {
        let id = self.next_id();
        let (items, receiver) = mpsc::unbounded_channel();
        let frame = request_frame(&id, tag, payload);
        let failure = Slot::default();
        self.request(
            &id,
            &frame,
            Waiter::Stream {
                items,
                frame: resend.then(|| frame.clone()),
                failure: failure.clone(),
            },
        )?;
        Ok(Subscription {
            items: receiver,
            id,
            link: self.link.clone(),
            failure,
        })
    }

    pub fn close(&self) {
        lock(&self.link).stopping = true;
    }
}

async fn keep(link: Link, origin: String, issue: Issue, mut served: JoinHandle<()>) {
    loop {
        let _ = served.await;
        lose_connection(&mut lock(&link));
        match reconnect(&origin, &issue, &link).await {
            Some(socket) => served = install(&link, socket),
            None => {
                close_for_good(&mut lock(&link));
                return;
            }
        }
    }
}

async fn reconnect(origin: &str, issue: &Issue, link: &Link) -> Option<Socket> {
    for delay in std::iter::once(Duration::ZERO).chain(RETRY_DELAYS) {
        tokio::time::sleep(delay).await;
        if lock(link).stopping {
            return None;
        }
        if let Ok(socket) = open(origin, issue).await {
            return Some(socket);
        }
    }
    None
}

fn install(link: &Link, socket: Socket) -> JoinHandle<()> {
    let (out, outgoing) = mpsc::unbounded_channel();
    {
        let mut state = lock(link);
        for waiter in state.waiters.values() {
            if let Waiter::Stream {
                frame: Some(frame), ..
            } = waiter
            {
                let _ = out.send(frame_message(frame));
            }
        }
        state.out = Some(out.clone());
    }
    tokio::spawn(serve(socket, outgoing, out, link.clone()))
}

fn lose_connection(state: &mut State) {
    state.out = None;
    for (id, waiter) in std::mem::take(&mut state.waiters) {
        match waiter {
            Waiter::Call(reply) => {
                let _ = reply.send(Err(LOST_CALL.to_owned()));
            }
            Waiter::Stream {
                frame: None,
                failure,
                ..
            } => {
                *lock(&failure) = Some(Failure::ConnectionLost);
            }
            stream => {
                state.waiters.insert(id, stream);
            }
        }
    }
}

fn close_for_good(state: &mut State) {
    state.closed = true;
    state.out = None;
    for waiter in std::mem::take(&mut state.waiters).into_values() {
        if let Waiter::Stream { failure, .. } = waiter {
            *lock(&failure) = Some(Failure::ConnectionLost);
        }
    }
}

async fn serve(
    socket: Socket,
    mut outgoing: mpsc::UnboundedReceiver<Message>,
    ack: mpsc::UnboundedSender<Message>,
    link: Link,
) {
    let (mut sink, mut source) = socket.split();
    let write = async {
        while let Some(message) = outgoing.recv().await {
            if sink.send(message).await.is_err() {
                break;
            }
        }
    };
    let read = async {
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
                dispatch(&link, &ack, frame);
            }
        }
    };
    tokio::select! {
        () = write => {}
        () = read => {}
    }
}

fn dispatch(link: &Link, ack: &mpsc::UnboundedSender<Message>, frame: Value) {
    let Some(request_id) = frame["requestId"].as_str() else {
        return;
    };
    match frame["_tag"].as_str() {
        Some("Chunk") => {
            let values = frame["values"].as_array().map_or(&[][..], Vec::as_slice);
            let state = lock(link);
            match state.waiters.get(request_id) {
                Some(Waiter::Stream { items, .. }) if !values.is_empty() => {
                    for (index, value) in values.iter().enumerate() {
                        let last = index + 1 == values.len();
                        let _ = items.send(Item {
                            value: value.clone(),
                            ack: last.then(|| ack.clone()),
                        });
                    }
                }
                _ => {
                    let _ = ack.send(frame_message(
                        &json!({"_tag": "Ack", "requestId": request_id}),
                    ));
                }
            }
        }
        Some("Exit") => {
            let result = if frame["exit"]["_tag"] == "Success" {
                Ok(frame["exit"]["value"].clone())
            } else {
                Err(cause_text(&frame["exit"]["cause"]))
            };
            match lock(link).waiters.remove(request_id) {
                Some(Waiter::Call(reply)) => {
                    let _ = reply.send(result);
                }
                Some(Waiter::Stream { failure, .. }) => {
                    if let Err(cause) = result {
                        *lock(&failure) = Some(Failure::Refused(cause));
                    }
                }
                None => {}
            }
        }
        _ => {}
    }
}

async fn open(origin: &str, issue: &Issue) -> Result<Socket, String> {
    let token = issue().await?;
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
    let (socket, _) = tokio::time::timeout(TIMEOUT, tokio_tungstenite::connect_async(request))
        .await
        .map_err(|_| format!("connect {origin}/ws: it did not answer in time"))?
        .map_err(|e| format!("connect {origin}/ws: {e}"))?;
    Ok(socket)
}

fn cause_text(cause: &Value) -> String {
    let causes = cause
        .as_array()
        .map_or(std::slice::from_ref(cause), Vec::as_slice);
    causes
        .iter()
        .map(|cause| {
            &cause[if cause["_tag"] == "Die" {
                "defect"
            } else {
                "error"
            }]
        })
        .find_map(|reason| reason["message"].as_str().or(reason.as_str()))
        .map_or_else(|| cause.to_string(), str::to_owned)
}

fn request_frame(id: &str, tag: &str, payload: Value) -> Value {
    json!({"_tag": "Request", "id": id, "tag": tag, "payload": payload, "headers": []})
}

fn frame_message(frame: &Value) -> Message {
    Message::Text(frame.to_string().into())
}

#[cfg(test)]
impl Client {
    pub(crate) fn waiting(&self) -> usize {
        lock(&self.link).waiters.len()
    }

    pub(crate) fn connected(&self) -> bool {
        lock(&self.link).out.is_some()
    }
}
