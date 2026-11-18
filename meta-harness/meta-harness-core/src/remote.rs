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
use crate::util::{IDLE, TIMEOUT, lock};
use crate::{
    Arrival, Automation, Context, GitActionRequest, GitProgress, GitStatus, HostProject, Landing,
    Lineage, LineageChanges, LocalServer, ModelChoice, MoveCheck, MoveFinish, MovePackage,
    MoveRecord, Permissions, Provider, PullRequestDetail, Refs, RequestId, ResolvedPullRequest,
    Snapshot, StartThread, Statistics, ThreadEvent, ThreadSummary, Turn, UsageLimits, Watch,
};

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
        let (socket, _) = tokio::time::timeout(TIMEOUT, tokio_tungstenite::connect_async(request))
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

    pub async fn status(&self) -> Result<api::Status, String> {
        self.call(Call::Host(HostCall::Status)).await
    }

    pub async fn providers(&self) -> Result<Vec<Provider>, String> {
        self.call(Call::Host(HostCall::Providers)).await
    }

    pub async fn environment_name(&self) -> Result<String, String> {
        self.call(Call::Host(HostCall::EnvironmentName)).await
    }

    pub async fn environment_machine(&self) -> Result<String, String> {
        self.call(Call::Host(HostCall::EnvironmentMachine)).await
    }

    pub async fn environment_id(&self) -> Result<String, String> {
        self.call(Call::Host(HostCall::EnvironmentId)).await
    }

    pub async fn projects(&self) -> Result<Vec<HostProject>, String> {
        self.call(Call::Host(HostCall::Projects)).await
    }

    pub async fn preflight(&self, root: &str, branch: &str) -> Result<Landing, String> {
        self.call(Call::Host(HostCall::Preflight {
            root: root.to_owned(),
            branch: branch.to_owned(),
        }))
        .await
    }

    pub async fn move_status(&self, transfer: &str) -> Result<Arrival, String> {
        self.call(Call::Host(HostCall::MoveStatus {
            transfer: transfer.to_owned(),
        }))
        .await
    }

    pub async fn move_abort(&self, transfer: &str) -> Result<Arrival, String> {
        self.call(Call::Host(HostCall::MoveAbort {
            transfer: transfer.to_owned(),
        }))
        .await
    }

    pub async fn add_project(&self, root: &str) -> Result<HostProject, String> {
        self.call(Call::Host(HostCall::AddProject {
            root: root.to_owned(),
        }))
        .await
    }

    pub async fn clone_project(&self, remote_url: &str, path: &str) -> Result<HostProject, String> {
        self.call(Call::Host(HostCall::CloneProject {
            remote_url: remote_url.to_owned(),
            path: path.to_owned(),
        }))
        .await
    }

    pub async fn writer_model(&self) -> Result<Option<ModelChoice>, String> {
        self.call(Call::Host(HostCall::WriterModel)).await
    }

    pub async fn set_writer_model(&self, choice: &ModelChoice) -> Result<(), String> {
        self.call(Call::Host(HostCall::SetWriterModel(choice.clone())))
            .await
    }

    pub async fn project(&self, root: &Path) -> Result<Project, String> {
        let project = Project {
            host: self.clone(),
            root: root.to_string_lossy().into_owned(),
        };
        project.threads().await?;
        Ok(project)
    }

    pub async fn config(&self) -> Result<Current, String> {
        self.call(Call::Config(ConfigCall::Current)).await
    }

    pub async fn propose(&self, summary: &str, changes: Vec<Change>) -> Result<u64, String> {
        self.call(Call::Config(ConfigCall::Propose {
            summary: summary.to_owned(),
            changes,
        }))
        .await
    }

    pub async fn propose_revert(&self, revision: u64) -> Result<u64, String> {
        self.call(Call::Config(ConfigCall::Revert(revision))).await
    }

    pub async fn pending(&self) -> Result<Vec<Proposal>, String> {
        self.call(Call::Config(ConfigCall::Pending)).await
    }

    pub async fn approve(&self, number: u64) -> Result<Current, String> {
        self.call(Call::Config(ConfigCall::Approve(number))).await
    }

    pub async fn reject(&self, number: u64) -> Result<(), String> {
        self.call(Call::Config(ConfigCall::Reject(number))).await
    }

    pub async fn log(&self) -> Result<Vec<LogEntry>, String> {
        self.call(Call::Config(ConfigCall::Log)).await
    }

    pub async fn remote_on(&self, port: u16) -> Result<RemoteStatus, String> {
        self.call(Call::Remote(RemoteCall::On { port })).await
    }

    pub async fn remote_off(&self) -> Result<(), String> {
        self.call(Call::Remote(RemoteCall::Off)).await
    }

    pub async fn remote_status(&self) -> Result<Option<RemoteStatus>, String> {
        self.call(Call::Remote(RemoteCall::Status)).await
    }

    pub async fn invite(&self, kind: ClientKind, access: Access) -> Result<String, String> {
        self.call(Call::Invite { kind, access }).await
    }

    pub async fn t3_pairing(&self) -> Result<String, String> {
        self.call(Call::Hosts(HostsCall::T3Pairing)).await
    }

    pub async fn revoke_client(&self, name: &str) -> Result<(), String> {
        self.call(Call::Hosts(HostsCall::Revoke(name.to_owned())))
            .await
    }

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

    pub async fn host_remove(&self, name: &str) -> Result<String, String> {
        self.call(Call::Hosts(HostsCall::Remove(name.to_owned())))
            .await
    }

    pub async fn hosts(&self) -> Result<HostList, String> {
        self.call(Call::Hosts(HostsCall::List)).await
    }

    pub async fn host_agents(&self, on: bool) -> Result<(), String> {
        self.call(Call::Hosts(HostsCall::Agents(on))).await
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
    let (mut socket, _) = tokio::time::timeout(TIMEOUT, tokio_tungstenite::connect_async(request))
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
            let text = serde_json::to_string(&api::Request::Cancel { cancel: self.id })
                .expect("requests serialize");
            let _ = self.link.out.send(Message::Text(text.into()));
        }
    }
}

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

    pub async fn threads(&self) -> Result<Vec<ThreadSummary>, String> {
        self.call(ProjectCall::Threads).await
    }

    pub async fn start(&self, start: StartThread) -> Result<Thread, String> {
        let id: String = self.call(ProjectCall::Start(start)).await?;
        Ok(self.thread(id))
    }

    pub async fn move_in(&self, package: MovePackage) -> Result<Thread, String> {
        let id: String = self.call(ProjectCall::MoveIn(package)).await?;
        Ok(self.thread(id))
    }

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

    pub fn git(&self) -> Git {
        Git {
            host: self.host.clone(),
            checkout: Checkout::Project(self.root.clone()),
        }
    }

    pub async fn only_thread_in_worktree(&self, thread: &ThreadSummary) -> Result<bool, String> {
        self.call(ProjectCall::OnlyThreadInWorktree(thread.clone()))
            .await
    }

    pub async fn delete(&self, thread: &ThreadSummary, worktree: bool) -> Result<(), String> {
        self.call(ProjectCall::Delete {
            thread: thread.clone(),
            worktree,
        })
        .await
    }

    pub async fn archive(&self, thread: &str, archived: bool) -> Result<(), String> {
        self.call(ProjectCall::Archive {
            thread: thread.to_owned(),
            archived,
        })
        .await
    }
}

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

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn git(&self) -> Git {
        let (root, id) = self.names();
        Git {
            host: self.host.clone(),
            checkout: Checkout::Thread { root, id },
        }
    }

    pub async fn subscribe(&self) -> Result<(Snapshot, Stream<ThreadEvent>), String> {
        let (root, id) = self.names();
        self.host.stream(api::Stream::Subscribe { root, id }).await
    }

    pub async fn servers(&self) -> Result<Watch<Vec<LocalServer>>, String> {
        let (root, id) = self.names();
        let (first, stream) = self.host.stream(api::Stream::Servers { root, id }).await?;
        Ok(Watch::from_stream(first, stream))
    }

    pub async fn automations(&self) -> Result<Watch<Vec<Automation>>, String> {
        let (root, id) = self.names();
        let (first, stream) = self
            .host
            .stream(api::Stream::Automations { root, id })
            .await?;
        Ok(Watch::from_stream(first, stream))
    }

    pub async fn lineage_changes(&self) -> Result<LineageChanges, String> {
        let (root, id) = self.names();
        let ((), stream) = self
            .host
            .stream(api::Stream::LineageChanges { root, id })
            .await?;
        Ok(LineageChanges::from_stream(stream))
    }
}

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
    title: Title -> String;
    worktree: Worktree -> Option<PathBuf>;
    ended: Ended -> bool;
    snapshot: Snapshot -> Snapshot;
    turns: Turns -> Vec<Turn>;
    statistics: Statistics -> Statistics;
    context: Context -> Option<Context>;
    model: Model -> Option<ModelChoice>;
    record: Record -> Value;
    stop: Stop -> ();
    usage_limits: UsageLimits -> Option<UsageLimits>;
    lineage: Lineage -> Lineage;
    merge_back: MergeBack -> ();
    disconnect: Disconnect -> ();
}

impl Thread {
    pub async fn move_check(&self) -> Result<MoveCheck, String> {
        self.call(ThreadCall::MoveCheck).await
    }

    pub async fn move_out(
        &self,
        transfer: &str,
        target_address: &str,
        target_name: &str,
        target_environment: &str,
        commit: bool,
        message: Option<String>,
    ) -> Result<MovePackage, String> {
        self.call(ThreadCall::MoveOut {
            transfer: transfer.to_owned(),
            target_address: target_address.to_owned(),
            target_name: target_name.to_owned(),
            target_environment: target_environment.to_owned(),
            commit,
            message,
        })
        .await
    }

    pub async fn move_finish(&self, transfer: &str, finish: MoveFinish) -> Result<(), String> {
        self.call(ThreadCall::MoveFinish {
            transfer: transfer.to_owned(),
            finish,
        })
        .await
    }

    pub async fn move_record(&self) -> Result<Option<MoveRecord>, String> {
        self.call(ThreadCall::MoveRecord).await
    }

    pub async fn send(&self, text: &str, steer: bool) -> Result<(), String> {
        self.call(ThreadCall::Send {
            text: text.to_owned(),
            steer,
        })
        .await
    }

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

    pub async fn revert(&self, turn: u64, restore_files: bool) -> Result<String, String> {
        self.call(ThreadCall::Revert {
            turn,
            restore_files,
        })
        .await
    }

    pub async fn fork(&self, turn: u64) -> Result<String, String> {
        self.call(ThreadCall::Fork(turn)).await
    }

    pub async fn set_permissions(&self, permissions: Permissions) -> Result<(), String> {
        self.call(ThreadCall::SetPermissions(permissions)).await
    }

    pub async fn set_model(&self, model: &ModelChoice) -> Result<(), String> {
        self.call(ThreadCall::SetModel(model.clone())).await
    }

    pub async fn send_queued_now(&self, turn: u64) -> Result<(), String> {
        self.call(ThreadCall::SendQueuedNow(turn)).await
    }

    pub async fn edit_queued(&self, turn: u64, text: &str) -> Result<(), String> {
        self.call(ThreadCall::EditQueued {
            turn,
            text: text.to_owned(),
        })
        .await
    }

    pub async fn cancel_queued(&self, turn: u64) -> Result<(), String> {
        self.call(ThreadCall::CancelQueued(turn)).await
    }

    pub async fn respond(&self, request: &RequestId, decision: &str) -> Result<(), String> {
        self.call(ThreadCall::Respond {
            request: request.0.clone(),
            decision: decision.to_owned(),
        })
        .await
    }

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

    pub async fn dismiss(&self, request: &RequestId) -> Result<(), String> {
        self.call(ThreadCall::Dismiss(request.0.clone())).await
    }

    pub async fn set_branch(&self, branch: &str, worktree: Option<&Path>) -> Result<(), String> {
        self.call(ThreadCall::SetBranch {
            branch: branch.to_owned(),
            worktree: worktree.map(|p| p.to_string_lossy().into_owned()),
        })
        .await
    }

    pub async fn stop_server(&self, pid: u32, port: u16) -> Result<(), String> {
        self.call(ThreadCall::StopServer { pid, port }).await
    }

    pub async fn set_automation_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        self.call(ThreadCall::SetAutomationEnabled {
            id: id.to_owned(),
            enabled,
        })
        .await
    }

    pub async fn run_automation_now(&self, id: &str) -> Result<(), String> {
        self.call(ThreadCall::RunAutomationNow(id.to_owned())).await
    }

    pub async fn stop_subagent(&self, subagent: &str) -> Result<(), String> {
        self.call(ThreadCall::StopSubagent(subagent.to_owned()))
            .await
    }
}

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

    pub async fn status(&self) -> Result<GitStatus, String> {
        self.call(GitCall::Status).await
    }

    pub async fn watch(&self) -> Result<Stream<GitStatus>, String> {
        let ((), stream) = self
            .host
            .stream(api::Stream::GitWatch(self.checkout.clone()))
            .await?;
        Ok(stream)
    }

    pub async fn refs(&self, query: &str, cursor: u64, limit: u64) -> Result<Refs, String> {
        self.call(GitCall::Refs {
            query: query.to_owned(),
            cursor,
            limit,
        })
        .await
    }

    pub async fn switch(&self, name: &str) -> Result<(), String> {
        self.call(GitCall::Switch(name.to_owned())).await
    }

    pub async fn create_branch(&self, name: &str) -> Result<(), String> {
        self.call(GitCall::CreateBranch(name.to_owned())).await
    }

    pub async fn init(&self) -> Result<(), String> {
        self.call(GitCall::Init).await
    }

    pub async fn pull(&self) -> Result<(String, String), String> {
        self.call(GitCall::Pull).await
    }

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

    pub async fn branch_diff(&self, ignore_whitespace: bool) -> Result<String, String> {
        self.call(GitCall::BranchDiff(ignore_whitespace)).await
    }

    pub async fn uncommitted_diff(&self, ignore_whitespace: bool) -> Result<String, String> {
        self.call(GitCall::UncommittedDiff(ignore_whitespace)).await
    }

    pub async fn resolve_pull_request(
        &self,
        reference: &str,
    ) -> Result<ResolvedPullRequest, String> {
        self.call(GitCall::ResolvePullRequest(reference.to_owned()))
            .await
    }

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

    pub async fn pull_request(&self, number: u64) -> Result<PullRequestDetail, String> {
        self.call(GitCall::PullRequest(number)).await
    }

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

    pub async fn pull_request_worktree(
        &self,
        reference: &str,
    ) -> Result<(String, Option<PathBuf>), String> {
        self.call(GitCall::PullRequestWorktree(reference.to_owned()))
            .await
    }
}
