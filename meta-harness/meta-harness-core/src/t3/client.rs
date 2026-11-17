//! T3 Code's client protocol, Effect RPC JSON frames over one WebSocket on `/ws`. T3 closes a
//! socket when its session expires, so the client reconnects with a fresh token and sends its open
//! subscriptions again; calls and actions are never sent twice (ADR-0292).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::util::lock;

/// The orchestration protocol this client speaks; T3 answers 426 to any other.
const ORCHESTRATION_PROTOCOL: u32 = 2;

/// The pauses before the reconnection attempts after the first, which comes at once. When every
/// attempt fails, the client closes for good.
const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(200),
    Duration::from_millis(500),
    Duration::from_secs(1),
];

/// The failure of a call that a lost connection left unanswered. T3 may have run it, so the call is
/// not sent again.
const LOST_CALL: &str = "T3 closed the connection before it answered; the call was not sent again";

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Issues an owner token, as `t3 auth session issue` does.
type Issue = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

enum Waiter {
    Call(oneshot::Sender<Result<Value, String>>),
    /// An open stream and, for a subscription, the request frame that opened it, which is sent
    /// again on a new connection. An action's stream has none: it ends with its connection, since
    /// T3 may have done part of it already.
    Stream {
        items: mpsc::UnboundedSender<Item>,
        frame: Option<Value>,
    },
}

/// One item of a stream. The last item of a chunk carries the connection that receives the chunk's
/// acknowledgement, which the consumer sends once it takes that item.
struct Item {
    value: Value,
    ack: Option<mpsc::UnboundedSender<Message>>,
}

/// The state that the client's handles and subscriptions share.
#[derive(Default)]
struct State {
    /// The open connection's writer; `None` while the client reconnects, and once it closed for good.
    out: Option<mpsc::UnboundedSender<Message>>,
    /// The requests awaiting T3's replies by id.
    waiters: HashMap<String, Waiter>,
    /// Whether the client closed for good: no request may wait any more.
    closed: bool,
    /// Whether `close` was called: a lost connection then ends the client instead.
    stopping: bool,
}

type Link = Arc<Mutex<State>>;

/// One client of a T3 server. It keeps its connection until `close`, and reconnects when T3 drops it.
#[derive(Clone)]
pub(crate) struct Client {
    link: Link,
    next: Arc<AtomicU64>,
}

/// An open stream request and its items, as T3 sends them. Dropped, it tells T3 to stop the request
/// (Effect RPC's `Interrupt`) and forgets it.
pub(crate) struct Subscription {
    items: mpsc::UnboundedReceiver<Item>,
    id: String,
    link: Link,
}

impl Subscription {
    /// The next item T3 sent; `None` once the request ended. Taking a chunk's last item
    /// acknowledges the chunk, so T3 sends the next one only after the consumer took this one.
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
        // A request that already ended has nothing left to stop. Forgotten here, it is not sent again
        // on a new connection; while the client reconnects, there is no connection to interrupt on.
        let open = state.waiters.remove(&self.id).is_some();
        if let (true, Some(out)) = (open, &state.out) {
            let frame = json!({"_tag": "Interrupt", "requestId": self.id});
            let _ = out.send(frame_message(&frame));
        }
    }
}

impl Client {
    /// Connects to `origin` (`http://127.0.0.1:<port>`) with a token from `issue`, and keeps the
    /// connection: when T3 closes it, the client reconnects with a new token (see [`keep`]).
    pub async fn connect(
        origin: &str,
        issue: impl Fn() -> Result<String, String> + Send + Sync + 'static,
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

    /// Registers `waiter` for request `id` and sends `frame`, as one step. Fails at once while no
    /// connection is open.
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

    /// Calls `tag` and returns its success value, or T3's failure as text.
    pub async fn call(&self, tag: &str, payload: Value) -> Result<Value, String> {
        let id = self.next_id();
        let (reply, answer) = oneshot::channel();
        self.request(&id, &request_frame(&id, tag, payload), Waiter::Call(reply))?;
        answer
            .await
            .map_err(|_| "T3 connection is closed".to_owned())?
    }

    /// Calls `tag` with `payload` as a command, which T3 identifies by a fresh `commandId`.
    pub async fn command(&self, tag: &str, mut payload: Value) -> Result<Value, String> {
        payload["commandId"] = json!(super::fresh_id("cmd"));
        self.call(tag, payload).await
    }

    /// Opens the subscription `tag`; each received item arrives in order, already acknowledged.
    /// A new connection opens it again.
    pub async fn stream(&self, tag: &str, payload: Value) -> Result<Subscription, String> {
        self.open_stream(tag, payload, true)
    }

    /// Starts the action `tag`, whose progress streams like a subscription's items. A lost
    /// connection ends it, and it is never sent again, as a call is not.
    pub async fn action(&self, tag: &str, payload: Value) -> Result<Subscription, String> {
        self.open_stream(tag, payload, false)
    }

    fn open_stream(&self, tag: &str, payload: Value, resend: bool) -> Result<Subscription, String> {
        let id = self.next_id();
        let (items, receiver) = mpsc::unbounded_channel();
        let frame = request_frame(&id, tag, payload);
        self.request(
            &id,
            &frame,
            Waiter::Stream {
                items,
                frame: resend.then(|| frame.clone()),
            },
        )?;
        Ok(Subscription {
            items: receiver,
            id,
            link: self.link.clone(),
        })
    }

    /// Ends the client as its T3 is about to stop: a lost connection then ends the client instead of
    /// reconnecting it.
    pub fn close(&self) {
        lock(&self.link).stopping = true;
    }
}

/// Keeps the client connected: serves `served`'s connection until it closes, then reconnects. When
/// no connection opens, or the client is stopping, the client closes for good.
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

/// Opens a connection after the last one was lost: at once, then after each of [`RETRY_DELAYS`].
/// `None` once the client is stopping or every attempt failed.
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

/// Makes `socket` the open connection and sends the open streams on it again. The returned task
/// serves the connection until it closes.
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

/// The connection is lost: its calls fail, and its streams wait for the next connection.
fn lose_connection(state: &mut State) {
    state.out = None;
    for (id, waiter) in std::mem::take(&mut state.waiters) {
        match waiter {
            Waiter::Call(reply) => {
                let _ = reply.send(Err(LOST_CALL.to_owned()));
            }
            // An action's stream ends here: dropping its sender ends its items.
            Waiter::Stream { frame: None, .. } => {}
            stream => {
                state.waiters.insert(id, stream);
            }
        }
    }
}

/// The client closed for good: no request waits any more, and each open stream ends.
fn close_for_good(state: &mut State) {
    state.closed = true;
    state.out = None;
    state.waiters.clear();
}

/// Writes what the client sends and reads what T3 sends, until either side ends.
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
    // Frames without a request, such as Pong, concern the connection only.
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
                // Nothing waits for these items, so the chunk is acknowledged at once.
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
                Err(frame["exit"]["cause"].to_string())
            };
            // An ended stream leaves the waiters, which ends its items.
            if let Some(Waiter::Call(reply)) = lock(link).waiters.remove(request_id) {
                let _ = reply.send(result);
            }
        }
        _ => {}
    }
}

/// Opens a socket to `origin` with a new owner token. Issuing runs a process, so it waits off the
/// async runtime.
async fn open(origin: &str, issue: &Issue) -> Result<Socket, String> {
    let issuer = issue.clone();
    let token = tokio::task::spawn_blocking(move || issuer())
        .await
        .map_err(|e| e.to_string())??;
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
    Ok(socket)
}

fn request_frame(id: &str, tag: &str, payload: Value) -> Value {
    json!({"_tag": "Request", "id": id, "tag": tag, "payload": payload, "headers": []})
}

fn frame_message(frame: &Value) -> Message {
    Message::Text(frame.to_string().into())
}

#[cfg(test)]
impl Client {
    /// How many requests still await T3's replies.
    pub(crate) fn waiting(&self) -> usize {
        lock(&self.link).waiters.len()
    }

    /// Whether a connection is open now.
    pub(crate) fn connected(&self) -> bool {
        lock(&self.link).out.is_some()
    }
}
