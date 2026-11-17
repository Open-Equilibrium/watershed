//! Meta-Harness's service (ADR-0286, ADR-0297): it owns a data folder, starts the pinned T3 Code
//! server there and serves the API (see [`crate::api`]) on a WebSocket at 127.0.0.1, as T3 Code
//! serves its own clients. Every connection must carry the bearer token the service writes, with
//! its address, into `connection.json`, which only the user can read.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::stream::unfold;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};
use tokio::task::{AbortHandle, JoinHandle};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

mod admit;
mod listener;

use crate::api::{
    self, Call, Checkout, ConfigCall, GitCall, HostCall, HostsCall, ProjectCall, Reply, Stream,
    ThreadCall,
};
use crate::config::{Homes, Proposed, Store, now_ms, provider_instances};
use crate::hosts::{self, Links};

use crate::util::IDLE;
use crate::{AcpAgent, Host, HostConfig, Project, Thread, tunnel};
use admit::{Admission, Caller, Listener, admit, header, revoked};

/// The file that tells clients where the service listens and which token it accepts.
pub const CONNECTION_FILE: &str = "connection.json";

/// Frames a connection's writer holds before a stream waits for the client to read.
const OUTGOING: usize = 256;

/// Threads a connection keeps followed after their last call; a stream keeps its own thread.
const FOLLOWED: usize = 32;

/// How many remote connections may be in their handshake at once; the excess is closed (ADR-0299).
const HANDSHAKES: usize = 32;

/// How long a new connection may take to complete its handshake.
const HANDSHAKE: std::time::Duration = std::time::Duration::from_secs(10);

/// How long a reply to a malformed request may wait for room in the outgoing queue.
const REPLY_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The refusal of a call or stream that the caller's kind does not make (ADR-0299).
const REFUSED_CALL: &str = "This client may not make that call.";

/// The refusal of a link while T3 refuses agents from other hosts here (ADR-0300).
const LINKS_OFF: &str =
    "Agents from other hosts are off here. Turn them on first: meta host agents on.";

/// Revocations a connection may still be behind on; one behind further ends (see [`revoked`]).
const REVOKED: usize = 64;

/// How often linked hosts are checked for links that are due (ADR-0288).
const RELINK_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);

/// Meta-Harness's data folder: `META_HARNESS_HOME`, which must be absolute, else `.meta-harness`
/// in the home folder, as T3 Code keeps `~/.t3`.
pub fn default_home() -> Result<PathBuf, String> {
    match std::env::var_os("META_HARNESS_HOME") {
        Some(home) if Path::new(&home).is_absolute() => Ok(PathBuf::from(home)),
        Some(_) => Err("META_HARNESS_HOME must be an absolute path.".to_owned()),
        None => std::env::home_dir()
            .map(|home| home.join(".meta-harness"))
            .ok_or_else(|| "The home folder is unknown; set META_HARNESS_HOME.".to_owned()),
    }
}

/// What a service runs on.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    /// Meta-Harness's data folder: its store, T3's data, the harness homes and the connection
    /// file. One service at a time owns it.
    pub home: PathBuf,
    /// The pinned `t3` executable (ADR-0281).
    pub t3: PathBuf,
    /// Agents T3 runs as local ACP commands.
    pub acp_agents: Vec<AcpAgent>,
}

/// A running service.
pub struct Service {
    /// `ws://127.0.0.1:<port>`.
    pub url: String,
    /// The bearer token every connection must send.
    pub token: String,
    host: Arc<Host>,
    shared: Arc<Shared>,
    accept: JoinHandle<()>,
    relink: JoinHandle<()>,
    retry: JoinHandle<()>,
    stopping: tokio::sync::watch::Sender<bool>,
    connection_file: PathBuf,
}

struct Shared {
    host: Arc<Host>,
    store: Store,
    homes: Homes,
    person_codex: PathBuf,
    token: String,
    /// Turns true when the service stops, which ends every connection.
    stopping: tokio::sync::watch::Receiver<bool>,
    /// Held until the last connection and call ends; a new service on the folder fails until then.
    _lock: std::fs::File,
    /// The name of each client revoked; its open connections end (ADR-0299).
    revoked: tokio::sync::broadcast::Sender<String>,
    /// This host's name for other hosts: T3's name for it, in lower case.
    name: String,
    /// Where the remote listener binds; `None` for the Tailscale addresses.
    remote_bind: Option<Vec<IpAddr>>,
    /// The remote listener while remote access is on.
    remote: std::sync::Mutex<listener::Remote>,
    /// Held while remote access is turned on or off, or bound again, so those changes do not
    /// interleave.
    remote_serial: tokio::sync::Mutex<()>,
    /// The hosts this one links to and their tunnels.
    links: Links,
    /// The handshake slots of the remote listener (see [`HANDSHAKES`]).
    handshakes: Arc<tokio::sync::Semaphore>,
    /// Held for reading by each sign-in a tunnel forwards until T3 answers it, and for writing while
    /// a client's T3 sessions end, so no sign-in in flight survives that (ADR-0301).
    sign_ins: Arc<tokio::sync::RwLock<()>>,
}

impl Service {
    /// Takes the data folder, writes the current configuration into the harness homes, starts
    /// T3 with Claude Code and Codex pointed at them and listens for clients. Remote access binds
    /// the Tailscale addresses.
    pub async fn start(config: ServiceConfig) -> Result<Self, String> {
        Self::start_with_remote(config, None).await
    }

    /// As [`Service::start`], but remote access binds `remote_bind` instead of the Tailscale
    /// addresses (ADR-0299); the tests bind loopback this way.
    pub async fn start_with_remote(
        config: ServiceConfig,
        remote_bind: Option<Vec<IpAddr>>,
    ) -> Result<Self, String> {
        create_private_dir(&config.home)?;
        let lock = take_lock(&config.home)?;
        let store = Store::open(&config.home.join("meta-harness.sqlite"))?;
        let homes = Homes::under(&config.home);
        let person_codex = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::home_dir().map(|h| h.join(".codex")))
            .ok_or("The home folder is unknown.")?;
        // A revision the homes cannot take must not keep the service from serving its revert.
        let written = homes.write(&store.current()?.blocks, &person_codex);
        if let Err(error) = &written {
            eprintln!("meta: {error}");
        }
        store.set_reconciled(written.is_ok());
        let t3_home = config.home.join("t3");
        let (links_t3, links_home) = (config.t3.clone(), t3_home.clone());
        let host = Arc::new(
            Host::start(HostConfig {
                binary: config.t3.clone(),
                home: t3_home.clone(),
                acp_agents: config.acp_agents,
            })
            .await?,
        );
        let name = host.environment_name().await?.to_lowercase();
        let links = Links::new(name.clone(), links_t3, links_home);
        let claude_dir = std::env::var("CLAUDE_CONFIG_DIR").ok();
        let instances = provider_instances(
            &host.provider_instances().await?,
            &homes,
            claude_dir.as_deref(),
        )?;
        host.set_provider_instances(instances).await?;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Could not listen on the loopback: {e}"))?;
        let url = format!("ws://{}", listener.local_addr().map_err(|e| e.to_string())?);
        let token = hosts::new_token();
        let connection_file = config.home.join(CONNECTION_FILE);
        write_private(
            &connection_file,
            &json!({"url": url, "token": token, "pid": std::process::id()}).to_string(),
        )?;
        let (stopping, stopped) = tokio::sync::watch::channel(false);
        let (revoked, _) = tokio::sync::broadcast::channel(REVOKED);
        let shared = Arc::new(Shared {
            host: host.clone(),
            store,
            homes,
            person_codex,
            token: token.clone(),
            stopping: stopped,
            _lock: lock,
            revoked,
            name,
            remote_bind,
            remote: std::sync::Mutex::new(listener::Remote::default()),
            remote_serial: tokio::sync::Mutex::default(),
            links,
            handshakes: Arc::new(tokio::sync::Semaphore::new(HANDSHAKES)),
            sign_ins: Arc::new(tokio::sync::RwLock::new(())),
        });
        // Remote access stays on across restarts (ADR-0299); a port it cannot take is retried.
        let remembered = hosts::remote_port(&shared.store.db())?;
        if let Some(port) = remembered
            && let Err(error) = listener::start(&shared, port).await
        {
            eprintln!("meta: remote access is not listening yet: {error}");
        }
        shared.links.start_tunnels(&shared.store).await?;
        let accept = tokio::spawn(accept(
            listener,
            shared.clone(),
            Listener::Local,
            shared.stopping.clone(),
        ));
        let relink = tokio::spawn(relink(shared.clone()));
        let retry = tokio::spawn(listener::retry(shared.clone()));
        Ok(Self {
            url,
            token,
            host,
            shared,
            accept,
            relink,
            retry,
            stopping,
            connection_file,
        })
    }

    /// Stops listening, ends every connection, removes the connection file and stops T3.
    pub async fn stop(self) {
        let host = self.host.clone();
        drop(self);
        host.shutdown().await;
    }
}

/// A dropped service stops as [`Service::stop`] does, except that it does not stop T3: T3's host
/// stops when its last handle is dropped.
impl Drop for Service {
    fn drop(&mut self) {
        self.accept.abort();
        self.relink.abort();
        self.retry.abort();
        self.shared.links.stop();
        // The remote listener's tasks hold the shared state, so they end with it.
        drop(crate::util::lock(&self.shared.remote).listening.take());
        let _ = self.stopping.send(true);
        let _ = std::fs::remove_file(&self.connection_file);
    }
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path)
        .map_err(|e| format!("Could not create {}: {e}", path.display()))?;
    #[cfg(unix)]
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .map_err(|e| format!("Could not protect {}: {e}", path.display()))?;
    Ok(())
}

/// Locks `home` for this service; a second service on it fails at once.
fn take_lock(home: &Path) -> Result<std::fs::File, String> {
    let path = home.join("service.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("Could not open {}: {e}", path.display()))?;
    file.try_lock()
        .map_err(|_| format!("Another Meta-Harness service runs on {}.", home.display()))?;
    Ok(file)
}

/// Writes `text` to `path` readable only by the user, replacing it whole.
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let temporary = path.with_extension("tmp");
    let _ = std::fs::remove_file(&temporary);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|e| format!("Could not write {}: {e}", temporary.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("Could not write {}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .map_err(|e| format!("Could not write {}: {e}", path.display()))
}

/// Makes the links that are due, now and every hour, until the service stops (ADR-0288).
async fn relink(shared: Arc<Shared>) {
    loop {
        shared.links.relink_due(&shared.store).await;
        tokio::time::sleep(RELINK_EVERY).await;
    }
}

/// Serves each connection that `listener` accepts, until the task ends.
async fn accept(
    listener: TcpListener,
    shared: Arc<Shared>,
    kind: Listener,
    ends: tokio::sync::watch::Receiver<bool>,
) {
    crate::util::accept_each(listener, move |socket| {
        // A remote connection needs one of the handshake slots; with none free it is closed.
        let slot = match kind {
            Listener::Local => Ok(None),
            Listener::Remote => shared.handshakes.clone().try_acquire_owned().map(Some),
        };
        let (shared, ends) = (shared.clone(), ends.clone());
        async move {
            if let Ok(slot) = slot {
                serve(shared, socket, kind, ends, slot).await;
            }
        }
    })
    .await;
}

/// One client connection: its handshake must admit it (see [`admit`]); then each request runs on
/// its own task, and streams end when the client cancels them or leaves.
// tungstenite's handshake callback returns its own large error response.
#[allow(clippy::result_large_err)]
async fn serve(
    shared: Arc<Shared>,
    socket: TcpStream,
    listener: Listener,
    ends: tokio::sync::watch::Receiver<bool>,
    slot: Option<tokio::sync::OwnedSemaphorePermit>,
) {
    let mut client = String::new();
    let mut admission = None;
    // Subscribed before the token is checked, so no revocation during the handshake is missed.
    let mut revocations = shared.revoked.subscribe();
    let check = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        admission = Some(admit(&shared, listener, request)?);
        client = hosts::printable(header(request, api::CLIENT_HEADER).unwrap_or("unnamed"));
        Ok(response)
    };
    let Ok(Ok(mut socket)) = tokio::time::timeout(
        HANDSHAKE,
        tokio_tungstenite::accept_hdr_async(socket, check),
    )
    .await
    else {
        return;
    };
    // The handshake is over, so its slot is free for the next connection.
    drop(slot);
    let (caller, owner) = match admission {
        Some(Admission::Session(caller, owner)) => (caller, owner),
        Some(Admission::Tunnel(owner, access)) => {
            let mut ends = ends.clone();
            tokio::select! {
                () = tunnel::serve(
                    socket,
                    shared.host.t3_address().to_owned(),
                    owner.clone(),
                    access,
                    shared.sign_ins.clone(),
                ) => {}
                () = revoked(&mut revocations, Some(&owner)) => {}
                _ = ends.wait_for(|stopped| *stopped) => {}
            }
            return;
        }
        Some(Admission::Paired(token, access)) => {
            let answer = serde_json::to_string(&api::Pairing {
                name: shared.name.clone(),
                token,
                access,
            })
            .expect("pairings serialize");
            let _ = socket.send(Message::Text(answer.into())).await;
            let _ = socket.close(None).await;
            return;
        }
        None => return,
    };
    let (mut sink, mut frames) = socket.split();
    let (out, mut outgoing) = mpsc::channel::<Message>(OUTGOING);
    // The connection's tasks are owned here, so when the connection ends, aborted or not, they end too
    // and nothing they hold outlives it (their state holds the service's data-folder lock).
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(async move {
        while let Some(message) = outgoing.recv().await {
            if sink.send(message).await.is_err() {
                break;
            }
        }
    });
    let mut stopping = ends;
    let session = Arc::new(Session {
        shared,
        client,
        caller,
        projects: Mutex::default(),
        threads: Mutex::default(),
    });
    let mut streams: HashMap<u64, AbortHandle> = HashMap::new();
    // Signalled by a malformed request's reply that the queue did not take in time.
    let (stalled, mut stalls) = mpsc::channel::<()>(1);
    // The one malformed request whose reply may wait for queue room.
    let replying = Arc::new(tokio::sync::Semaphore::new(1));
    let mut ping = crate::util::pings();
    let mut heard = tokio::time::Instant::now();
    loop {
        // The ping is sent after the select, so no guard it holds lives across that await.
        let frame = tokio::select! {
            frame = frames.next() => Some(frame),
            _ = stopping.wait_for(|stopped| *stopped) => break,
            () = revoked(&mut revocations, owner.as_deref()) => break,
            _ = stalls.recv() => break,
            _ = ping.tick() => None,
        };
        let Some(frame) = frame else {
            if heard.elapsed() > IDLE {
                break;
            }
            // A full queue is a peer that is not reading, which the next ping finds.
            let _ = out.try_send(Message::Ping(Vec::new().into()));
            continue;
        };
        let Some(Ok(frame)) = frame else { break };
        heard = tokio::time::Instant::now();
        let Message::Text(text) = frame else { continue };
        let request = match serde_json::from_str::<api::Request>(&text) {
            Ok(request) => request,
            // Such as a call a newer client knows and this service does not: it fails at once.
            Err(error) => {
                let id = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| v["id"].as_u64());
                if let Some(id) = id {
                    let error = format!("Meta-Harness does not understand this request: {error}");
                    // A second reply while one waits means a client that does not read: it is
                    // disconnected. The reply waits on a task of its own, so the loop goes on
                    // noticing revocation and shutdown; one that takes no reply within
                    // REPLY_WAIT is disconnected too.
                    let Ok(permit) = replying.clone().try_acquire_owned() else {
                        break;
                    };
                    let (out, stalled) = (out.clone(), stalled.clone());
                    tasks.spawn(async move {
                        let _permit = permit;
                        let reply = frame_of(&Reply::Error { id, error });
                        if tokio::time::timeout(REPLY_WAIT, out.send(reply))
                            .await
                            .is_err()
                        {
                            let _ = stalled.try_send(());
                        }
                    });
                }
                continue;
            }
        };
        while tasks.try_join_next().is_some() {}
        streams.retain(|_, task| !task.is_finished());
        match request {
            api::Request::Call { id, call } => {
                let (session, out) = (session.clone(), out.clone());
                tasks.spawn(async move {
                    let reply = match session.call(call).await {
                        Ok(ok) => Reply::Ok { id, ok },
                        Err(error) => Reply::Error { id, error },
                    };
                    let _ = out.send(frame_of(&reply)).await;
                });
            }
            api::Request::Stream { id, stream } => {
                let (session, out) = (session.clone(), out.clone());
                let task = tasks.spawn(async move {
                    if let Err(error) = session.stream(id, stream, &out).await {
                        let _ = out.send(frame_of(&Reply::Error { id, error })).await;
                    }
                });
                // A client that reuses a live stream's id ends that stream, so none is left
                // running without a way to cancel it.
                if let Some(earlier) = streams.insert(id, task) {
                    earlier.abort();
                }
            }
            api::Request::Cancel { cancel } => {
                if let Some(task) = streams.remove(&cancel) {
                    task.abort();
                }
            }
        }
    }
    tasks.abort_all();
}

fn frame_of(reply: &Reply) -> Message {
    Message::Text(
        serde_json::to_string(reply)
            .expect("replies serialize")
            .into(),
    )
}

/// Answers stream `id` with `first`, then sends each of `items` until the client is gone or the
/// items end, and ends the stream.
async fn answer(
    out: &mpsc::Sender<Message>,
    id: u64,
    first: Value,
    items: impl futures_util::Stream<Item = Result<Value, String>>,
) {
    let _ = out.send(frame_of(&Reply::Ok { id, ok: first })).await;
    let mut items = std::pin::pin!(items);
    while let Some(item) = items.next().await {
        let reply = match item {
            Ok(item) => Reply::Item { id, item },
            Err(error) => Reply::Error { id, error },
        };
        if out.send(frame_of(&reply)).await.is_err() {
            return;
        }
    }
    let _ = out.send(frame_of(&Reply::End { id, end: () })).await;
}

fn to(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// What one connection opened: projects by folder and threads by id.
struct Session {
    shared: Arc<Shared>,
    client: String,
    caller: Caller,
    projects: Mutex<HashMap<String, Project>>,
    /// Followed threads, the most recently used last.
    threads: Mutex<Vec<(String, Thread)>>,
}

impl Session {
    fn host(&self) -> &Host {
        &self.shared.host
    }

    async fn project(&self, root: &str) -> Result<Project, String> {
        let mut projects = self.projects.lock().await;
        if let Some(project) = projects.get(root) {
            return Ok(project.clone());
        }
        let project = self.host().project(Path::new(root)).await?;
        projects.insert(root.to_owned(), project.clone());
        Ok(project)
    }

    /// The thread `id`, followed on first use, also after the client reconnected.
    async fn thread(&self, root: &str, id: &str) -> Result<Thread, String> {
        {
            let mut threads = self.threads.lock().await;
            if let Some(at) = threads.iter().position(|(known, _)| known == id) {
                let entry = threads.remove(at);
                let thread = entry.1.clone();
                threads.push(entry);
                return Ok(thread);
            }
        }
        let thread = self.project(root).await?.open(id).await?;
        self.keep(thread.clone()).await;
        Ok(thread)
    }

    /// Keeps `thread` followed, letting go of the least recently used beyond [`FOLLOWED`].
    async fn keep(&self, thread: Thread) {
        let mut threads = self.threads.lock().await;
        threads.retain(|(known, _)| known != thread.id());
        threads.push((thread.id().to_owned(), thread));
        let excess = threads.len().saturating_sub(FOLLOWED);
        threads.drain(..excess);
    }

    async fn git(&self, checkout: &Checkout) -> Result<crate::Git, String> {
        Ok(match checkout {
            Checkout::Project(root) => self.project(root).await?.git(),
            Checkout::Thread { root, id } => self.thread(root, id).await?.git(),
        })
    }

    async fn call(&self, call: Call) -> Result<Value, String> {
        if !self.caller.answers(&call) {
            return Err(REFUSED_CALL.to_owned());
        }
        match call {
            Call::Host(call) => self.host_call(call).await,
            Call::Project { root, call } => self.project_call(&root, call).await,
            Call::Thread { root, id, call } => {
                let thread = self.thread(&root, &id).await?;
                thread_call(&thread, call).await
            }
            Call::Git { checkout, call } => git_call(&self.git(&checkout).await?, call).await,
            Call::Config(call) => self.config_call(call),
            Call::Remote(call) => listener::call(&self.shared, call).await,
            Call::Invite { kind, access } => to(hosts::invite(
                &self.shared.store.db(),
                kind,
                access,
                now_ms(),
            )?),
            Call::Hosts(call) => self.hosts_call(call).await,
        }
    }

    async fn hosts_call(&self, call: HostsCall) -> Result<Value, String> {
        let shared = &self.shared;
        match call {
            HostsCall::T3Pairing => to(shared.links.pairing_code(self.caller.access()).await?),
            HostsCall::Revoke(name) => {
                // The client goes first, so none of its connections is admitted again. Then the sign-ins
                // in flight finish, its open connections end with its token (ADR-0299), and the sessions
                // its sign-ins made end (ADR-0301). A failure there leaves the name to revoke again.
                let had_client = hosts::revoke(&shared.store.db(), &name)?;
                let ended = {
                    let _sign_ins = shared.sign_ins.write().await;
                    let _ = shared.revoked.send(name.clone());
                    shared.links.end_sessions(&name).await
                };
                match ended {
                    Ok(ended) if had_client || ended > 0 => to(()),
                    Ok(_) => Err(format!("There is no client named {name}.")),
                    Err(error) => Err(format!(
                        "{error} The client's T3 sessions did not all end; run \"meta host revoke {name}\" again."
                    )),
                }
            }
            HostsCall::Add {
                address,
                code,
                access,
            } => {
                if !shared.host.linked_environments().await? {
                    return Err(LINKS_OFF.to_owned());
                }
                to(shared
                    .links
                    .add(&shared.store, &address, &code, access)
                    .await?)
            }
            HostsCall::List => to(shared.links.list(&shared.store).await?),
            HostsCall::Remove(name) => to(shared.links.remove(&shared.store, &name).await?),
            HostsCall::Agents(on) => to(shared.host.set_linked_environments(on).await?),
        }
    }

    async fn host_call(&self, call: HostCall) -> Result<Value, String> {
        let host = self.host();
        match call {
            HostCall::Status => to(api::Status {
                t3_version: crate::t3_version().to_owned(),
                pid: std::process::id(),
                revision: self.shared.store.current()?.revision,
                reconciled: self.shared.store.reconciled(),
            }),
            HostCall::Providers => to(host.providers().await?),
            HostCall::EnvironmentName => to(host.environment_name().await?),
            HostCall::WriterModel => to(host.writer_model().await?),
            HostCall::SetWriterModel(choice) => to(host.set_writer_model(&choice).await?),
        }
    }

    async fn project_call(&self, root: &str, call: ProjectCall) -> Result<Value, String> {
        let project = self.project(root).await?;
        match call {
            ProjectCall::Threads => to(project.threads().await?),
            ProjectCall::Start(start) => {
                let thread = project.start(start).await?;
                let id = thread.id().to_owned();
                self.keep(thread).await;
                to(id)
            }
            ProjectCall::Open(id) => to(self.thread(root, &id).await.map(|_| ())?),
            ProjectCall::OnlyThreadInWorktree(thread) => {
                to(project.only_thread_in_worktree(&thread).await?)
            }
            ProjectCall::Delete { thread, worktree } => {
                self.threads
                    .lock()
                    .await
                    .retain(|(known, _)| *known != thread.id);
                to(project.delete(&thread, worktree).await?)
            }
            ProjectCall::Archive { thread, archived } => {
                to(project.archive(&thread, archived).await?)
            }
        }
    }

    fn config_call(&self, call: ConfigCall) -> Result<Value, String> {
        let (store, client) = (&self.shared.store, self.client.as_str());
        match call {
            ConfigCall::Current => to(store.current()?),
            ConfigCall::Propose { summary, changes } => {
                to(store.propose(client, &summary, Proposed::Changes(changes))?)
            }
            ConfigCall::Revert(revision) => to(store.propose(
                client,
                &format!("Back to revision {revision}"),
                Proposed::Restore(revision),
            )?),
            ConfigCall::Pending => to(store.pending()?),
            ConfigCall::Approve(number) => {
                let shared = &self.shared;
                to(store.approve(client, number, |blocks| {
                    shared.homes.write(blocks, &shared.person_codex)
                })?)
            }
            ConfigCall::Reject(number) => to(store.reject(client, number)?),
            ConfigCall::Log => to(store.log()?),
        }
    }

    /// Answers stream `id` with its first value, then sends its items until it ends.
    async fn stream(
        &self,
        id: u64,
        stream: Stream,
        out: &mpsc::Sender<Message>,
    ) -> Result<(), String> {
        if !self.caller.streams() {
            return Err(REFUSED_CALL.to_owned());
        }
        match stream {
            Stream::Subscribe { root, id: thread } => {
                // The stream holds the thread, so it stays followed however many others open.
                let followed = self.thread(&root, &thread).await?;
                let (snapshot, events) = followed.subscribe();
                let items = unfold(events, |mut events| async move {
                    Some((to(events.recv().await?), events))
                });
                answer(out, id, to(snapshot)?, items).await;
            }
            Stream::Servers { root, id: thread } => {
                let watch = self.thread(&root, &thread).await?.servers().await?;
                let first = to(watch.current())?;
                let items = unfold(watch, |mut watch| async move {
                    Some((to(watch.changed().await?), watch))
                });
                answer(out, id, first, items).await;
            }
            Stream::Automations { root, id: thread } => {
                let watch = self.thread(&root, &thread).await?.automations().await?;
                let first = to(watch.current())?;
                let items = unfold(watch, |mut watch| async move {
                    Some((to(watch.changed().await?), watch))
                });
                answer(out, id, first, items).await;
            }
            Stream::LineageChanges { root, id: thread } => {
                let changes = self.thread(&root, &thread).await?.lineage_changes().await?;
                let items = unfold(changes, |changes| async move {
                    changes
                        .changed()
                        .await
                        .then_some((Ok(Value::Null), changes))
                });
                answer(out, id, Value::Null, items).await;
            }
            Stream::GitWatch(checkout) => {
                let statuses = self.git(&checkout).await?.watch().await?;
                let items = unfold(statuses, |mut statuses| async move {
                    Some((to(statuses.recv().await?), statuses))
                });
                answer(out, id, Value::Null, items).await;
            }
            Stream::GitRun { checkout, request } => {
                let progress = self.git(&checkout).await?.run(request).await?;
                let items = unfold(progress, |mut progress| async move {
                    Some((to(progress.recv().await?), progress))
                });
                answer(out, id, Value::Null, items).await;
            }
        }
        Ok(())
    }
}

async fn thread_call(thread: &Thread, call: ThreadCall) -> Result<Value, String> {
    use crate::RequestId;
    match call {
        ThreadCall::Title => to(thread.title()),
        ThreadCall::Worktree => to(thread.worktree()),
        ThreadCall::Ended => to(thread.ended()),
        ThreadCall::Snapshot => to(thread.snapshot()),
        ThreadCall::Turns => to(thread.turns()),
        ThreadCall::Statistics => to(thread.statistics()),
        ThreadCall::Context => to(thread.context()),
        ThreadCall::Model => to(thread.model()),
        ThreadCall::Record => to(thread.record()),
        ThreadCall::Send { text, steer } => to(thread.send(&text, steer).await?),
        ThreadCall::Stop => to(thread.stop().await?),
        ThreadCall::Diff {
            from,
            to: until,
            ignore_whitespace,
        } => to(thread.diff(from, until, ignore_whitespace).await?),
        ThreadCall::Revert {
            turn,
            restore_files,
        } => to(thread.revert(turn, restore_files).await?),
        ThreadCall::Fork(turn) => to(thread.fork(turn).await?),
        ThreadCall::SetPermissions(permissions) => to(thread.set_permissions(permissions).await?),
        ThreadCall::SetModel(model) => to(thread.set_model(&model).await?),
        ThreadCall::SendQueuedNow(turn) => to(thread.send_queued_now(turn).await?),
        ThreadCall::EditQueued { turn, text } => to(thread.edit_queued(turn, &text).await?),
        ThreadCall::CancelQueued(turn) => to(thread.cancel_queued(turn).await?),
        ThreadCall::Respond { request, decision } => {
            to(thread.respond(&RequestId(request), &decision).await?)
        }
        ThreadCall::Answer { request, answers } => {
            to(thread.answer(&RequestId(request), &answers).await?)
        }
        ThreadCall::Dismiss(request) => to(thread.dismiss(&RequestId(request)).await?),
        ThreadCall::SetBranch { branch, worktree } => to(thread
            .set_branch(&branch, worktree.as_deref().map(Path::new))
            .await?),
        ThreadCall::StopServer { pid, port } => to(thread.stop_server(pid, port).await?),
        ThreadCall::SetAutomationEnabled { id, enabled } => {
            to(thread.set_automation_enabled(&id, enabled).await?)
        }
        ThreadCall::RunAutomationNow(id) => to(thread.run_automation_now(&id).await?),
        ThreadCall::UsageLimits => to(thread.usage_limits().await?),
        ThreadCall::Lineage => to(thread.lineage().await?),
        ThreadCall::MergeBack => to(thread.merge_back().await?),
        ThreadCall::Disconnect => to(thread.disconnect().await?),
        ThreadCall::StopSubagent(id) => to(thread.stop_subagent(&id).await?),
    }
}

async fn git_call(git: &crate::Git, call: GitCall) -> Result<Value, String> {
    match call {
        GitCall::Status => to(git.status().await?),
        GitCall::Refs {
            query,
            cursor,
            limit,
        } => to(git.refs(&query, cursor, limit).await?),
        GitCall::Switch(name) => to(git.switch(&name).await?),
        GitCall::CreateBranch(name) => to(git.create_branch(&name).await?),
        GitCall::Init => to(git.init().await?),
        GitCall::Pull => to(git.pull().await?),
        GitCall::BranchDiff(ignore) => to(git.branch_diff(ignore).await?),
        GitCall::UncommittedDiff(ignore) => to(git.uncommitted_diff(ignore).await?),
        GitCall::ResolvePullRequest(reference) => to(git.resolve_pull_request(&reference).await?),
        GitCall::CheckOutPullRequest {
            reference,
            worktree,
        } => to(git.check_out_pull_request(&reference, worktree).await?),
        GitCall::Publish {
            provider,
            repository,
            public,
        } => to(git.publish(&provider, &repository, public).await?),
        GitCall::PullRequest(number) => to(git.pull_request(number).await?),
        GitCall::RunPullRequestAction {
            number,
            action,
            merge_method,
        } => to(git
            .run_pull_request_action(number, &action, merge_method.as_deref())
            .await?),
        GitCall::PullRequestWorktree(reference) => {
            to(git.pull_request_worktree(&reference).await?)
        }
    }
}
