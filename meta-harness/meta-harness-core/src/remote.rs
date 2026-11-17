//! A client of the Meta-Harness service (ADR-0215, ADR-0297): the same host, projects, threads
//! and Git as the core offers, reached over the service's WebSocket. A lost connection fails the
//! calls it left unanswered, which are never sent again, ends its streams, and the next call
//! connects anew, reading the connection file again when the client was made from it.

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

use crate::api::{
    self, Call, Checkout, ConfigCall, GitCall, HostCall, HostsCall, ProjectCall, RemoteCall,
    RemoteStatus, Reply, ThreadCall,
};
use crate::config::{Change, Current, LogEntry, Proposal};
use crate::hosts::{Access, ClientKind, HostList};
use crate::util::{TIMEOUT, lock};
use crate::{
    Automation, Context, GitActionRequest, GitProgress, GitStatus, Lineage, LineageChanges,
    LocalServer, ModelChoice, Permissions, Provider, PullRequestDetail, Refs, RequestId,
    ResolvedPullRequest, Snapshot, StartThread, Statistics, ThreadEvent, ThreadSummary, Turn,
    UsageLimits, Watch,
};

/// Items a stream holds for a reader that falls behind; past that the stream ends, as a
/// thread's own subscription does (ADR-0272, ADR-0290).
const BACKLOG: usize = 4096;

/// The failure of a call that a lost connection left unanswered. The service may have run it, so
/// it is not sent again.
const LOST_CALL: &str =
    "Meta-Harness closed the connection before it answered; the call was not sent again";

/// How the errors of a lost or missing connection start (see [`connection_lost`]).
const CONNECTION_ERRORS: [&str; 3] = [
    "Meta-Harness closed the connection",
    "Meta-Harness is not running",
    "Could not reach Meta-Harness",
];

/// Whether `error`, from a call or a stream of this client, says the connection to Meta-Harness is
/// lost or missing, so that a later attempt may succeed. Any other error came from Meta-Harness
/// itself or from its connection file, and a later attempt would get it again.
pub fn connection_lost(error: &str) -> bool {
    CONNECTION_ERRORS
        .iter()
        .any(|start| error.starts_with(start))
}

/// The error of a service that refuses this client's API version, which names the version the
/// service speaks. It is not a lost connection: a later attempt would be refused again.
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

/// Where the service is.
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

/// One open connection.
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

/// A connection to the Meta-Harness service on this host.
#[derive(Clone)]
pub struct Host {
    inner: Arc<Inner>,
}

impl Host {
    /// Connects to the service whose data folder is `home`, through its connection file;
    /// `client` names this client in the configuration log.
    pub async fn connect(home: &Path, client: &str) -> Result<Self, String> {
        Self::open(
            Endpoint::File(home.join(crate::service::CONNECTION_FILE)),
            client,
        )
        .await
    }

    /// Connects to the service at `url` with `token`.
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

    /// The open connection, connecting again when the last one was lost.
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
        let (socket, _) = tokio::time::timeout(TIMEOUT, tokio_tungstenite::connect_async(request))
            .await
            .map_err(|_| format!("Could not reach Meta-Harness at {url}: it did not answer in time."))?
            .map_err(|e| match e {
                // A service that stopped without removing its connection file.
                tokio_tungstenite::tungstenite::Error::Io(io)
                    if io.kind() == std::io::ErrorKind::ConnectionRefused =>
                {
                    format!(
                        "Meta-Harness is not running: nothing answers at {url}. Start it with \
                         \"meta serve\", or \"meta service install\" for the login service; its log \
                         is service.log in the data folder."
                    )
                }
                // A service of another API version refuses this client, naming its version.
                tokio_tungstenite::tungstenite::Error::Http(response)
                    if response.status() == StatusCode::UPGRADE_REQUIRED =>
                {
                    refused_version(&response)
                }
                e => format!("Could not reach Meta-Harness at {url}: {e}"),
            })?;
        let (mut sink, mut frames) = socket.split();
        let (out, mut outgoing) = mpsc::unbounded_channel::<Message>();
        let waiters: Arc<Mutex<Option<HashMap<u64, Waiter>>>> =
            Arc::new(Mutex::new(Some(HashMap::new())));
        tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                if sink.send(message).await.is_err() {
                    break;
                }
            }
            // The last handle is gone: closing the sink ends the connection for the reader too.
            let _ = sink.close().await;
        });
        let read = waiters.clone();
        // Held weakly, so dropping every handle of this client closes the connection.
        let cancel = out.downgrade();
        tokio::spawn(async move {
            while let Some(Ok(frame)) = frames.next().await {
                if let Message::Text(text) = frame
                    && let Ok(reply) = serde_json::from_str::<Reply>(&text)
                {
                    deliver(&read, &cancel, reply);
                }
            }
            // The connection is gone: every waiting call fails and every stream ends.
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

    /// What the service runs.
    pub async fn status(&self) -> Result<api::Status, String> {
        self.call(Call::Host(HostCall::Status)).await
    }

    /// The providers T3 offers, with their models.
    pub async fn providers(&self) -> Result<Vec<Provider>, String> {
        self.call(Call::Host(HostCall::Providers)).await
    }

    /// The name T3 shows for this host.
    pub async fn environment_name(&self) -> Result<String, String> {
        self.call(Call::Host(HostCall::EnvironmentName)).await
    }

    /// The writer model (ADR-0294), if one is set.
    pub async fn writer_model(&self) -> Result<Option<ModelChoice>, String> {
        self.call(Call::Host(HostCall::WriterModel)).await
    }

    /// Sets the writer model.
    pub async fn set_writer_model(&self, choice: &ModelChoice) -> Result<(), String> {
        self.call(Call::Host(HostCall::SetWriterModel(choice.clone())))
            .await
    }

    /// The T3 project for the folder `root`, created when T3 has none yet.
    pub async fn project(&self, root: &Path) -> Result<Project, String> {
        let project = Project {
            host: self.clone(),
            root: root.to_string_lossy().into_owned(),
        };
        project.threads().await?;
        Ok(project)
    }

    /// The current configuration revision.
    pub async fn config(&self) -> Result<Current, String> {
        self.call(Call::Config(ConfigCall::Current)).await
    }

    /// Proposes `changes` and returns the proposal's number; nothing changes before approval.
    pub async fn propose(&self, summary: &str, changes: Vec<Change>) -> Result<u64, String> {
        self.call(Call::Config(ConfigCall::Propose {
            summary: summary.to_owned(),
            changes,
        }))
        .await
    }

    /// Proposes going back to `revision`.
    pub async fn propose_revert(&self, revision: u64) -> Result<u64, String> {
        self.call(Call::Config(ConfigCall::Revert(revision))).await
    }

    /// The proposals that wait for a decision.
    pub async fn pending(&self) -> Result<Vec<Proposal>, String> {
        self.call(Call::Config(ConfigCall::Pending)).await
    }

    /// Approves proposal `number`; the service writes the new revision into the harness homes.
    pub async fn approve(&self, number: u64) -> Result<Current, String> {
        self.call(Call::Config(ConfigCall::Approve(number))).await
    }

    /// Rejects proposal `number`.
    pub async fn reject(&self, number: u64) -> Result<(), String> {
        self.call(Call::Config(ConfigCall::Reject(number))).await
    }

    /// The configuration log, oldest first.
    pub async fn log(&self) -> Result<Vec<LogEntry>, String> {
        self.call(Call::Config(ConfigCall::Log)).await
    }

    /// Turns remote access on at `port` (`meta remote on`), answering where it listens.
    pub async fn remote_on(&self, port: u16) -> Result<RemoteStatus, String> {
        self.call(Call::Remote(RemoteCall::On { port })).await
    }

    /// Turns remote access off (`meta remote off`).
    pub async fn remote_off(&self) -> Result<(), String> {
        self.call(Call::Remote(RemoteCall::Off)).await
    }

    /// Where the remote listener answers, while it is on.
    pub async fn remote_status(&self) -> Result<Option<RemoteStatus>, String> {
        self.call(Call::Remote(RemoteCall::Status)).await
    }

    /// A pairing code for a new client of `kind`, valid for five minutes, which gives a host
    /// client `access` (`meta invite`).
    pub async fn invite(&self, kind: ClientKind, access: Access) -> Result<String, String> {
        self.call(Call::Invite { kind, access }).await
    }

    /// A pairing code from this host's T3 Code, which another host's link redeems (ADR-0288).
    pub async fn t3_pairing(&self) -> Result<String, String> {
        self.call(Call::Hosts(HostsCall::T3Pairing)).await
    }

    /// Revokes the client named `name`, whose token is refused from its next connection on.
    pub async fn revoke_client(&self, name: &str) -> Result<(), String> {
        self.call(Call::Hosts(HostsCall::Revoke(name.to_owned())))
            .await
    }

    /// Links the host at `address` (`ws://…`) with `code` from its `meta invite --host`, at
    /// `access` or the level the other host allows, and answers the linked host's name and level
    /// (`meta host add`).
    pub async fn host_add(
        &self,
        address: &str,
        code: &str,
        access: Option<Access>,
    ) -> Result<(String, Access), String> {
        self.call(Call::Hosts(HostsCall::Add {
            address: address.to_owned(),
            code: code.to_owned(),
            access,
        }))
        .await
    }

    /// Unlinks the host named `name` (`meta host remove`) and answers the name this host paired
    /// under at that host, which its `meta host revoke` names.
    pub async fn host_remove(&self, name: &str) -> Result<String, String> {
        self.call(Call::Hosts(HostsCall::Remove(name.to_owned())))
            .await
    }

    /// The hosts this one links to and the clients paired with it (`meta host list`).
    pub async fn hosts(&self) -> Result<HostList, String> {
        self.call(Call::Hosts(HostsCall::List)).await
    }

    /// Turns agents from other hosts on or off (`meta host agents`).
    pub async fn host_agents(&self, on: bool) -> Result<(), String> {
        self.call(Call::Hosts(HostsCall::Agents(on))).await
    }
}

/// The request for a WebSocket handshake to `url` that carries `headers`.
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

/// Pairs this client with the remote listener at `address` (`ws://…`), using `code`, a pairing code
/// that the other side issued. Answers the other side's name, the token this client uses from now
/// on and the access it was paired at (ADR-0299, ADR-0300).
pub async fn pair(address: &str, code: &str, client: &str) -> Result<api::Pairing, String> {
    let request = handshake(
        address,
        &[(api::PAIRING_HEADER, code), (api::CLIENT_HEADER, client)],
    )?;
    let (mut socket, _) = tokio::time::timeout(TIMEOUT, tokio_tungstenite::connect_async(request))
        .await
        .map_err(|_| format!("Meta-Harness at {address} did not answer in time."))?
        .map_err(|e| match e {
            // The refusal says why in its body, such as a code that is wrong or used.
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

/// The reason a service gave for refusing a handshake, from its body.
fn refusal(response: &Response<Option<Vec<u8>>>) -> String {
    let body = response.body().as_deref().unwrap_or_default();
    let reason = String::from_utf8_lossy(body).trim().to_owned();
    if reason.is_empty() {
        format!("Meta-Harness refused the pairing ({}).", response.status())
    } else {
        reason
    }
}

fn send(link: &Link, id: u64, waiter: Waiter, request: &api::Request) -> Result<(), String> {
    let mut waiters = lock(&link.waiters);
    let waiters = waiters
        .as_mut()
        .ok_or("Meta-Harness closed the connection")?;
    waiters.insert(id, waiter);
    let text = serde_json::to_string(request).expect("requests serialize");
    link.out.send(Message::Text(text.into())).map_err(|_| {
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
            // A reader that fell behind, or left, ends the stream.
            if full {
                waiters.remove(&id);
                let text = serde_json::to_string(&api::Request::Cancel { cancel: id })
                    .expect("requests serialize");
                if let Some(out) = out.upgrade() {
                    let _ = out.send(Message::Text(text.into()));
                }
            }
        }
        Reply::End { id, .. } => {
            waiters.remove(&id);
        }
    }
}

/// The items of a stream. Dropped, it tells the service to stop sending them.
pub struct Stream<T> {
    receiver: mpsc::Receiver<Value>,
    id: u64,
    link: Arc<Link>,
    items: std::marker::PhantomData<fn() -> T>,
}

impl<T: DeserializeOwned> Stream<T> {
    /// The next item; `None` once the stream ended, also when the connection was lost or the
    /// reader fell too far behind.
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
            let text = serde_json::to_string(&api::Request::Cancel { cancel: self.id })
                .expect("requests serialize");
            let _ = self.link.out.send(Message::Text(text.into()));
        }
    }
}

/// A project on the service's host.
#[derive(Clone)]
pub struct Project {
    host: Host,
    root: String,
}

impl Project {
    async fn call<T: DeserializeOwned>(&self, call: ProjectCall) -> Result<T, String> {
        self.host
            .call(Call::Project {
                root: self.root.clone(),
                call,
            })
            .await
    }

    /// The project's threads.
    pub async fn threads(&self) -> Result<Vec<ThreadSummary>, String> {
        self.call(ProjectCall::Threads).await
    }

    /// Starts a thread with its first prompt.
    pub async fn start(&self, start: StartThread) -> Result<Thread, String> {
        let id: String = self.call(ProjectCall::Start(start)).await?;
        Ok(self.thread(id))
    }

    /// Opens the thread `id`.
    pub async fn open(&self, id: &str) -> Result<Thread, String> {
        self.call::<()>(ProjectCall::Open(id.to_owned())).await?;
        Ok(self.thread(id.to_owned()))
    }

    fn thread(&self, id: String) -> Thread {
        Thread {
            host: self.host.clone(),
            root: self.root.clone(),
            id,
        }
    }

    /// Git for the project folder.
    pub fn git(&self) -> Git {
        Git {
            host: self.host.clone(),
            checkout: Checkout::Project(self.root.clone()),
        }
    }

    /// Whether `thread` is the only thread in its worktree.
    pub async fn only_thread_in_worktree(&self, thread: &ThreadSummary) -> Result<bool, String> {
        self.call(ProjectCall::OnlyThreadInWorktree(thread.clone()))
            .await
    }

    /// Deletes `thread`, and its worktree with `worktree`.
    pub async fn delete(&self, thread: &ThreadSummary, worktree: bool) -> Result<(), String> {
        self.call(ProjectCall::Delete {
            thread: thread.clone(),
            worktree,
        })
        .await
    }

    /// Archives or restores the thread `thread`.
    pub async fn archive(&self, thread: &str, archived: bool) -> Result<(), String> {
        self.call(ProjectCall::Archive {
            thread: thread.to_owned(),
            archived,
        })
        .await
    }
}

/// A thread on the service's host.
#[derive(Clone)]
pub struct Thread {
    host: Host,
    root: String,
    id: String,
}

impl Thread {
    async fn call<T: DeserializeOwned>(&self, call: ThreadCall) -> Result<T, String> {
        self.host
            .call(Call::Thread {
                root: self.root.clone(),
                id: self.id.clone(),
                call,
            })
            .await
    }

    fn names(&self) -> (String, String) {
        (self.root.clone(), self.id.clone())
    }

    /// T3's thread id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Git for the thread's worktree, else its project folder, as it is at each call.
    pub fn git(&self) -> Git {
        let (root, id) = self.names();
        Git {
            host: self.host.clone(),
            checkout: Checkout::Thread { root, id },
        }
    }

    /// The current snapshot and the events after it, with no gap between them (ADR-0290).
    pub async fn subscribe(&self) -> Result<(Snapshot, Stream<ThreadEvent>), String> {
        let (root, id) = self.names();
        self.host.stream(api::Stream::Subscribe { root, id }).await
    }

    /// The thread's local servers, kept current.
    pub async fn servers(&self) -> Result<Watch<Vec<LocalServer>>, String> {
        let (root, id) = self.names();
        let (first, stream) = self.host.stream(api::Stream::Servers { root, id }).await?;
        Ok(Watch::from_stream(first, stream))
    }

    /// The thread's automations, kept current.
    pub async fn automations(&self) -> Result<Watch<Vec<Automation>>, String> {
        let (root, id) = self.names();
        let (first, stream) = self
            .host
            .stream(api::Stream::Automations { root, id })
            .await?;
        Ok(Watch::from_stream(first, stream))
    }

    /// Wakes when the thread's lineage may have changed.
    pub async fn lineage_changes(&self) -> Result<LineageChanges, String> {
        let (root, id) = self.names();
        let ((), stream) = self
            .host
            .stream(api::Stream::LineageChanges { root, id })
            .await?;
        Ok(LineageChanges::from_stream(stream))
    }
}

/// Calls without arguments that read a thread.
macro_rules! reads {
    ($($(#[$doc:meta])* $name:ident: $call:ident -> $out:ty;)*) => {
        impl Thread {
            $($(#[$doc])* pub async fn $name(&self) -> Result<$out, String> {
                self.call(ThreadCall::$call).await
            })*
        }
    };
}

reads! {
    /// Its title.
    title: Title -> String;
    /// Its own worktree, if it has one.
    worktree: Worktree -> Option<PathBuf>;
    /// Whether T3 ended its stream for good.
    ended: Ended -> bool;
    /// The thread as it is now.
    snapshot: Snapshot -> Snapshot;
    /// Its turns, oldest first.
    turns: Turns -> Vec<Turn>;
    /// Its statistics.
    statistics: Statistics -> Statistics;
    /// Its latest context window use.
    context: Context -> Option<Context>;
    /// Its model.
    model: Model -> Option<ModelChoice>;
    /// T3's record of it.
    record: Record -> Value;
    /// Stops the running turn.
    stop: Stop -> ();
    /// The usage limits of its provider.
    usage_limits: UsageLimits -> Option<UsageLimits>;
    /// Its lineage.
    lineage: Lineage -> Lineage;
    /// Merges this fork back into its source.
    merge_back: MergeBack -> ();
    /// Disconnects it from its parent.
    disconnect: Disconnect -> ();
}

impl Thread {
    /// Sends `text`, as a steer with `steer` while a turn runs.
    pub async fn send(&self, text: &str, steer: bool) -> Result<(), String> {
        self.call(ThreadCall::Send {
            text: text.to_owned(),
            steer,
        })
        .await
    }

    /// The diff from turn `from` to turn `to`.
    pub async fn diff(
        &self,
        from: u64,
        to: u64,
        ignore_whitespace: bool,
    ) -> Result<String, String> {
        self.call(ThreadCall::Diff {
            from,
            to,
            ignore_whitespace,
        })
        .await
    }

    /// Reverts to before turn `turn`.
    pub async fn revert(&self, turn: u64, restore_files: bool) -> Result<String, String> {
        self.call(ThreadCall::Revert {
            turn,
            restore_files,
        })
        .await
    }

    /// Forks the thread after turn `turn` and returns the fork's id.
    pub async fn fork(&self, turn: u64) -> Result<String, String> {
        self.call(ThreadCall::Fork(turn)).await
    }

    /// Sets what the agent may do without asking.
    pub async fn set_permissions(&self, permissions: Permissions) -> Result<(), String> {
        self.call(ThreadCall::SetPermissions(permissions)).await
    }

    /// Sets the model.
    pub async fn set_model(&self, model: &ModelChoice) -> Result<(), String> {
        self.call(ThreadCall::SetModel(model.clone())).await
    }

    /// Sends the queued turn `turn` now.
    pub async fn send_queued_now(&self, turn: u64) -> Result<(), String> {
        self.call(ThreadCall::SendQueuedNow(turn)).await
    }

    /// Changes the text of the queued turn `turn`.
    pub async fn edit_queued(&self, turn: u64, text: &str) -> Result<(), String> {
        self.call(ThreadCall::EditQueued {
            turn,
            text: text.to_owned(),
        })
        .await
    }

    /// Cancels the queued turn `turn`.
    pub async fn cancel_queued(&self, turn: u64) -> Result<(), String> {
        self.call(ThreadCall::CancelQueued(turn)).await
    }

    /// Answers an approval request with `decision`.
    pub async fn respond(&self, request: &RequestId, decision: &str) -> Result<(), String> {
        self.call(ThreadCall::Respond {
            request: request.0.clone(),
            decision: decision.to_owned(),
        })
        .await
    }

    /// Answers a question request.
    pub async fn answer(
        &self,
        request: &RequestId,
        answers: &[(String, Vec<String>)],
    ) -> Result<(), String> {
        self.call(ThreadCall::Answer {
            request: request.0.clone(),
            answers: answers.to_vec(),
        })
        .await
    }

    /// Dismisses a request.
    pub async fn dismiss(&self, request: &RequestId) -> Result<(), String> {
        self.call(ThreadCall::Dismiss(request.0.clone())).await
    }

    /// Points the thread at `branch` and, if given, `worktree`.
    pub async fn set_branch(&self, branch: &str, worktree: Option<&Path>) -> Result<(), String> {
        self.call(ThreadCall::SetBranch {
            branch: branch.to_owned(),
            worktree: worktree.map(|p| p.to_string_lossy().into_owned()),
        })
        .await
    }

    /// Stops the local server `pid` on `port`.
    pub async fn stop_server(&self, pid: u32, port: u16) -> Result<(), String> {
        self.call(ThreadCall::StopServer { pid, port }).await
    }

    /// Pauses or resumes the automation `id`.
    pub async fn set_automation_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        self.call(ThreadCall::SetAutomationEnabled {
            id: id.to_owned(),
            enabled,
        })
        .await
    }

    /// Runs the automation `id` now.
    pub async fn run_automation_now(&self, id: &str) -> Result<(), String> {
        self.call(ThreadCall::RunAutomationNow(id.to_owned())).await
    }

    /// Stops the subagent thread `subagent`.
    pub async fn stop_subagent(&self, subagent: &str) -> Result<(), String> {
        self.call(ThreadCall::StopSubagent(subagent.to_owned()))
            .await
    }
}

/// Git for a checkout on the service's host.
#[derive(Clone)]
pub struct Git {
    host: Host,
    checkout: Checkout,
}

impl Git {
    async fn call<T: DeserializeOwned>(&self, call: GitCall) -> Result<T, String> {
        self.host
            .call(Call::Git {
                checkout: self.checkout.clone(),
                call,
            })
            .await
    }

    /// The checkout's status.
    pub async fn status(&self) -> Result<GitStatus, String> {
        self.call(GitCall::Status).await
    }

    /// Each new status of the checkout.
    pub async fn watch(&self) -> Result<Stream<GitStatus>, String> {
        let ((), stream) = self
            .host
            .stream(api::Stream::GitWatch(self.checkout.clone()))
            .await?;
        Ok(stream)
    }

    /// A page of refs matching `query`.
    pub async fn refs(&self, query: &str, cursor: u64, limit: u64) -> Result<Refs, String> {
        self.call(GitCall::Refs {
            query: query.to_owned(),
            cursor,
            limit,
        })
        .await
    }

    /// Switches to `name`.
    pub async fn switch(&self, name: &str) -> Result<(), String> {
        self.call(GitCall::Switch(name.to_owned())).await
    }

    /// Creates and switches to the branch `name`.
    pub async fn create_branch(&self, name: &str) -> Result<(), String> {
        self.call(GitCall::CreateBranch(name.to_owned())).await
    }

    /// Makes the folder a repository.
    pub async fn init(&self) -> Result<(), String> {
        self.call(GitCall::Init).await
    }

    /// Pulls, answering T3's title and description.
    pub async fn pull(&self) -> Result<(String, String), String> {
        self.call(GitCall::Pull).await
    }

    /// Runs a stacked action and reports its progress.
    pub async fn run(&self, request: GitActionRequest) -> Result<Stream<GitProgress>, String> {
        let ((), stream) = self
            .host
            .stream(api::Stream::GitRun {
                checkout: self.checkout.clone(),
                request,
            })
            .await?;
        Ok(stream)
    }

    /// The branch's changes against its base.
    pub async fn branch_diff(&self, ignore_whitespace: bool) -> Result<String, String> {
        self.call(GitCall::BranchDiff(ignore_whitespace)).await
    }

    /// The uncommitted changes.
    pub async fn uncommitted_diff(&self, ignore_whitespace: bool) -> Result<String, String> {
        self.call(GitCall::UncommittedDiff(ignore_whitespace)).await
    }

    /// Resolves a pull request reference.
    pub async fn resolve_pull_request(
        &self,
        reference: &str,
    ) -> Result<ResolvedPullRequest, String> {
        self.call(GitCall::ResolvePullRequest(reference.to_owned()))
            .await
    }

    /// Checks out a pull request, in a new worktree with `worktree`.
    pub async fn check_out_pull_request(
        &self,
        reference: &str,
        worktree: bool,
    ) -> Result<(String, Option<PathBuf>), String> {
        self.call(GitCall::CheckOutPullRequest {
            reference: reference.to_owned(),
            worktree,
        })
        .await
    }

    /// Publishes the repository.
    pub async fn publish(
        &self,
        provider: &str,
        repository: &str,
        public: bool,
    ) -> Result<(), String> {
        self.call(GitCall::Publish {
            provider: provider.to_owned(),
            repository: repository.to_owned(),
            public,
        })
        .await
    }

    /// The pull request `number` with its checks.
    pub async fn pull_request(&self, number: u64) -> Result<PullRequestDetail, String> {
        self.call(GitCall::PullRequest(number)).await
    }

    /// Runs a pull request action such as merging.
    pub async fn run_pull_request_action(
        &self,
        number: u64,
        action: &str,
        merge_method: Option<&str>,
    ) -> Result<(), String> {
        self.call(GitCall::RunPullRequestAction {
            number,
            action: action.to_owned(),
            merge_method: merge_method.map(str::to_owned),
        })
        .await
    }

    /// The branch and worktree of a pull request's checkout.
    pub async fn pull_request_worktree(
        &self,
        reference: &str,
    ) -> Result<(String, Option<PathBuf>), String> {
        self.call(GitCall::PullRequestWorktree(reference.to_owned()))
            .await
    }
}
