use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::{HeaderValue, Response, StatusCode};

use crate::api::{self, Call, Reply};
use crate::util::{IDLE, TIMEOUT, lock};

mod calls;
mod handles;

pub use handles::{Git, Project, Thread};

const BACKLOG: usize = 4096;

const LOST_CALL: &str =
    "Meta-Harness closed the connection before it answered; the call was not sent again";

const CONNECTION_ERRORS: [&str; 3] = [
    "Meta-Harness closed the connection",
    "Meta-Harness is not running",
    "Could not reach Meta-Harness",
];

pub fn connection_lost(error: &str) -> bool {
    CONNECTION_ERRORS
        .iter()
        .any(|start| error.starts_with(start))
}

fn refused_version(response: &Response<Option<Vec<u8>>>) -> String {
    let speaks = match response
        .headers()
        .get(api::API_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        Some(version) => format!("API version {version}"),
        None => "another API version".to_owned(),
    };
    format!(
        "Meta-Harness speaks {speaks}; this client needs {}. Update Meta-Harness or Liquid.",
        api::API_VERSION
    )
}

#[derive(Clone)]
enum Endpoint {
    Fixed { url: String, token: String },
    File(PathBuf),
}

impl Endpoint {
    fn resolve(&self) -> Result<(String, String), String> {
        match self {
            Self::Fixed { url, token } => Ok((url.clone(), token.clone())),
            Self::File(path) => {
                let text = std::fs::read_to_string(path).map_err(|_| {
                    format!(
                        "Meta-Harness is not running: {} is missing. Install Meta-Harness. Start \
                         it with \"meta serve\", or \"meta service install\" for the login service; \
                         its log is service.log in the data folder.",
                        path.display()
                    )
                })?;
                let value: Value = serde_json::from_str(&text)
                    .map_err(|e| format!("{} is damaged: {e}", path.display()))?;
                match (value["url"].as_str(), value["token"].as_str()) {
                    (Some(url), Some(token)) => Ok((url.to_owned(), token.to_owned())),
                    _ => Err(format!("{} names no address and token.", path.display())),
                }
            }
        }
    }
}

enum Waiter {
    Call(oneshot::Sender<Result<Value, String>>),
    Stream {
        first: Option<oneshot::Sender<Result<Value, String>>>,
        items: mpsc::Sender<Value>,
    },
}

struct Link {
    out: mpsc::UnboundedSender<Message>,
    waiters: Arc<Mutex<Option<HashMap<u64, Waiter>>>>,
}

struct Inner {
    endpoint: Endpoint,
    client: String,
    link: tokio::sync::Mutex<Option<Arc<Link>>>,
    next: AtomicU64,
}

#[derive(Clone)]
pub struct Host {
    inner: Arc<Inner>,
}

impl Host {
    pub async fn connect(home: &Path, client: &str) -> Result<Self, String> {
        Self::open(
            Endpoint::File(home.join(crate::service::CONNECTION_FILE)),
            client,
        )
        .await
    }

    pub async fn connect_to(url: &str, token: &str, client: &str) -> Result<Self, String> {
        Self::open(
            Endpoint::Fixed {
                url: url.to_owned(),
                token: token.to_owned(),
            },
            client,
        )
        .await
    }

    async fn open(endpoint: Endpoint, client: &str) -> Result<Self, String> {
        let host = Self {
            inner: Arc::new(Inner {
                endpoint,
                client: client.to_owned(),
                link: tokio::sync::Mutex::default(),
                next: AtomicU64::new(1),
            }),
        };
        host.link().await?;
        Ok(host)
    }

    async fn link(&self) -> Result<Arc<Link>, String> {
        let mut slot = self.inner.link.lock().await;
        if let Some(link) = slot.as_ref()
            && lock(&link.waiters).is_some()
        {
            return Ok(link.clone());
        }
        let (url, token) = self.inner.endpoint.resolve()?;
        let bearer = format!("Bearer {token}");
        let request = handshake(
            &url,
            &[
                ("authorization", bearer.as_str()),
                (api::CLIENT_HEADER, self.inner.client.as_str()),
            ],
        )?;
        let (socket, _) = tokio::time::timeout(TIMEOUT, tokio_tungstenite::connect_async_with_config(
            request,
            Some(crate::api::socket_config()),
            false,
        ))
            .await
            .map_err(|_| format!("Could not reach Meta-Harness at {url}: it did not answer in time."))?
            .map_err(|e| match e {
                tokio_tungstenite::tungstenite::Error::Io(io)
                    if io.kind() == std::io::ErrorKind::ConnectionRefused =>
                {
                    format!(
                        "Meta-Harness is not running: nothing answers at {url}. Start it with \
                         \"meta serve\", or \"meta service install\" for the login service; its log \
                         is service.log in the data folder."
                    )
                }
                tokio_tungstenite::tungstenite::Error::Http(response)
                    if response.status() == StatusCode::UPGRADE_REQUIRED =>
                {
                    refused_version(&response)
                }
                tokio_tungstenite::tungstenite::Error::Http(response)
                    if response.status() == StatusCode::UNAUTHORIZED =>
                {
                    format!(
                        "Meta-Harness at {url} no longer accepts this device's pairing; pair this \
                         device again."
                    )
                }
                e => format!("Could not reach Meta-Harness at {url}: {e}"),
            })?;
        let (mut sink, mut frames) = socket.split();
        let (out, mut outgoing) = mpsc::unbounded_channel::<Message>();
        let waiters: Arc<Mutex<Option<HashMap<u64, Waiter>>>> =
            Arc::new(Mutex::new(Some(HashMap::new())));
        let writer = tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                if sink.send(message).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });
        let read = waiters.clone();
        let cancel = out.downgrade();
        tokio::spawn(async move {
            while let Ok(Some(Ok(frame))) = tokio::time::timeout(IDLE, frames.next()).await {
                if let Message::Text(text) = frame
                    && let Ok(reply) = serde_json::from_str::<Reply>(&text)
                {
                    deliver(&read, &cancel, reply);
                }
            }
            writer.abort();
            for (_, waiter) in lock(&read).take().into_iter().flatten() {
                match waiter {
                    Waiter::Call(answer) => drop(answer.send(Err(LOST_CALL.to_owned()))),
                    Waiter::Stream {
                        first: Some(first), ..
                    } => {
                        drop(first.send(Err(LOST_CALL.to_owned())));
                    }
                    Waiter::Stream { .. } => {}
                }
            }
        });
        let link = Arc::new(Link { out, waiters });
        *slot = Some(link.clone());
        Ok(link)
    }

    async fn call<T: DeserializeOwned>(&self, call: Call) -> Result<T, String> {
        let link = self.link().await?;
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        send(
            &link,
            id,
            Waiter::Call(answer),
            &api::Request::Call { id, call },
        )?;
        let _waiter = WaiterGuard {
            link: link.clone(),
            id,
        };
        let value = answered.await.map_err(|_| LOST_CALL.to_owned())??;
        serde_json::from_value(value)
            .map_err(|e| format!("Meta-Harness answered unexpectedly: {e}"))
    }

    async fn stream<F: DeserializeOwned, T: DeserializeOwned>(
        &self,
        stream: api::Stream,
    ) -> Result<(F, Stream<T>), String> {
        let link = self.link().await?;
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let (first, answered) = oneshot::channel();
        let (items, receiver) = mpsc::channel(BACKLOG);
        let waiter = Waiter::Stream {
            first: Some(first),
            items,
        };
        send(&link, id, waiter, &api::Request::Stream { id, stream })?;
        let stream = Stream {
            receiver,
            id,
            link: link.clone(),
            items: std::marker::PhantomData,
        };
        let value = answered.await.map_err(|_| LOST_CALL.to_owned())??;
        let first = serde_json::from_value(value)
            .map_err(|e| format!("Meta-Harness answered unexpectedly: {e}"))?;
        Ok((first, stream))
    }
}

#[cfg(test)]
impl Host {
    pub(crate) async fn waiting(&self) -> usize {
        let slot = self.inner.link.lock().await;
        slot.as_ref()
            .and_then(|link| lock(&link.waiters).as_ref().map(HashMap::len))
            .unwrap_or(0)
    }
}

pub(crate) fn handshake(url: &str, headers: &[(&'static str, &str)]) -> Result<Request, String> {
    let version = api::API_VERSION.to_string();
    let mut request = url
        .into_client_request()
        .map_err(|e| format!("Bad Meta-Harness address {url}: {e}"))?;
    for (name, value) in headers
        .iter()
        .copied()
        .chain([(api::API_HEADER, version.as_str())])
    {
        let value =
            HeaderValue::from_str(value).map_err(|_| format!("{name} is not a header value."))?;
        request.headers_mut().insert(name, value);
    }
    Ok(request)
}

pub fn check_address(address: &str) -> Result<(), String> {
    if address.starts_with("ws://") {
        Ok(())
    } else {
        Err("Enter the address as ws://HOST:PORT, as meta invite prints it.".to_owned())
    }
}

pub async fn pair(address: &str, code: &str, client: &str) -> Result<api::Pairing, String> {
    check_address(address)?;
    let request = handshake(
        address,
        &[(api::PAIRING_HEADER, code), (api::CLIENT_HEADER, client)],
    )?;
    let (mut socket, _) = tokio::time::timeout(
        TIMEOUT,
        tokio_tungstenite::connect_async_with_config(
            request,
            Some(crate::api::socket_config()),
            false,
        ),
    )
    .await
    .map_err(|_| format!("Meta-Harness at {address} did not answer in time."))?
    .map_err(|e| match e {
        tokio_tungstenite::tungstenite::Error::Http(response) => refusal(&response),
        e => format!("Could not reach Meta-Harness at {address}: {e}"),
    })?;
    let answer = tokio::time::timeout(TIMEOUT, async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => return Some(text),
                Some(Ok(_)) => {}
                _ => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(|| format!("Meta-Harness at {address} did not answer the pairing."))?;
    let _ = socket.close(None).await;
    serde_json::from_str(&answer).map_err(|e| format!("Meta-Harness answered unexpectedly: {e}"))
}

fn refusal(response: &Response<Option<Vec<u8>>>) -> String {
    let body = response.body().as_deref().unwrap_or_default();
    let reason = String::from_utf8_lossy(body).trim().to_owned();
    if reason.is_empty() {
        format!("Meta-Harness refused the pairing ({}).", response.status())
    } else {
        reason
    }
}

fn frame(request: &api::Request) -> Message {
    Message::Text(
        serde_json::to_string(request)
            .expect("requests serialize")
            .into(),
    )
}

fn send(link: &Link, id: u64, waiter: Waiter, request: &api::Request) -> Result<(), String> {
    let mut waiters = lock(&link.waiters);
    let waiters = waiters
        .as_mut()
        .ok_or("Meta-Harness closed the connection")?;
    waiters.insert(id, waiter);
    link.out.send(frame(request)).map_err(|_| {
        waiters.remove(&id);
        "Meta-Harness closed the connection".to_owned()
    })
}

fn deliver(
    waiters: &Mutex<Option<HashMap<u64, Waiter>>>,
    out: &mpsc::WeakUnboundedSender<Message>,
    reply: Reply,
) {
    let mut guard = lock(waiters);
    let Some(waiters) = guard.as_mut() else {
        return;
    };
    match reply {
        Reply::Ok { id, ok } => match waiters.get_mut(&id) {
            Some(Waiter::Stream { first, .. }) => {
                if let Some(first) = first.take() {
                    let _ = first.send(Ok(ok));
                }
            }
            Some(Waiter::Call(_)) => {
                if let Some(Waiter::Call(answer)) = waiters.remove(&id) {
                    let _ = answer.send(Ok(ok));
                }
            }
            None => {}
        },
        Reply::Error { id, error } => match waiters.remove(&id) {
            Some(Waiter::Call(answer)) => drop(answer.send(Err(error))),
            Some(Waiter::Stream {
                first: Some(first), ..
            }) => drop(first.send(Err(error))),
            _ => {}
        },
        Reply::Item { id, item } => {
            let full = match waiters.get(&id) {
                Some(Waiter::Stream { items, .. }) => items.try_send(item).is_err(),
                _ => false,
            };
            if full {
                waiters.remove(&id);
                if let Some(out) = out.upgrade() {
                    let _ = out.send(frame(&api::Request::Cancel { cancel: id }));
                }
            }
        }
        Reply::End { id, .. } => {
            waiters.remove(&id);
        }
    }
}

struct WaiterGuard {
    link: Arc<Link>,
    id: u64,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        if let Some(waiters) = lock(&self.link.waiters).as_mut() {
            waiters.remove(&self.id);
        }
    }
}

pub struct Stream<T> {
    receiver: mpsc::Receiver<Value>,
    id: u64,
    link: Arc<Link>,
    items: std::marker::PhantomData<fn() -> T>,
}

impl<T: DeserializeOwned> Stream<T> {
    pub async fn recv(&mut self) -> Option<T> {
        let value = self.receiver.recv().await?;
        serde_json::from_value(value).ok()
    }
}

impl<T> Drop for Stream<T> {
    fn drop(&mut self) {
        let open = lock(&self.link.waiters)
            .as_mut()
            .is_some_and(|w| w.remove(&self.id).is_some());
        if open {
            let _ = self
                .link
                .out
                .send(frame(&api::Request::Cancel { cancel: self.id }));
        }
    }
}
