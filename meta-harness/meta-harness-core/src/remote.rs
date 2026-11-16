//! A client of the Meta-Harness service (ADR-0215, ADR-0297): the same host, projects, threads
//! and Git as the core offers, reached over the service's WebSocket. A lost connection fails the
//! calls it left unanswered, which are never sent again, ends its streams, and the next call
//! connects anew, reading the connection file again when the client was made from it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::api::{
    self, Call, Checkout, ConfigCall, GitCall, HostCall, ProjectCall, Reply, ThreadCall,
};
use crate::config::{Change, Current, LogEntry, Proposal};
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
                        "No Meta-Harness service runs: {} is missing. Install Meta-Harness or \
                         start it with `meta serve`.",
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
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
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| format!("Bad Meta-Harness address {url}: {e}"))?;
        let headers = request.headers_mut();
        headers.insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| "The Meta-Harness token is not a header value.".to_owned())?,
        );
        headers.insert(
            api::CLIENT_HEADER,
            self.inner
                .client
                .parse()
                .map_err(|_| "The client name is not a header value.".to_owned())?,
        );
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| format!("Could not reach Meta-Harness at {url}: {e}"))?;
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
